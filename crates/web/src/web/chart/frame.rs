//! `FiggyChart`'s frame loop, readiness waits, resize and prewarm.

use super::*;

#[wasm_bindgen]
impl FiggyChart {
    // ---- frame / resize / export ----

    /// Wait until previously submitted GPU work has finished. Does not
    /// draw. Used to separate an earlier compute submit from the next
    /// render submit when measuring startup.
    pub async fn wait_submitted_work(&self) -> Result<(), JsValue> {
        self.renderer.wait_submitted_work().await.map_err(js_err)
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
    /// freeze; it only reports first-submit completion. While a series'
    /// pipelines still compile in the background its frame leaves it out
    /// (#299); this draws again as they resolve and settles with the frame
    /// that holds every series.
    ///
    /// JS: `await chart.first_frame_ready();`
    pub async fn first_frame_ready(&mut self) -> Result<(), JsValue> {
        self.draw_until_complete().await?;
        renderer::init::wait_browser_submitted_work(self.renderer.queue())
            .await
            .map_err(js_err)?;
        Ok(())
    }

    /// Draw until a frame draws every series (#299). A frame that left a
    /// series out while its pipelines compile waits for a compile to change
    /// state, then draws again. A frame leaves a series out only while one
    /// of its compiles is waiting or compiling, and every compile ends
    /// (ready, rejected, device lost, superseded or cancelled), so this ends.
    async fn draw_until_complete(&mut self) -> Result<(), JsValue> {
        self.frame()?;
        while !self.renderer.frame_complete() {
            if self.renderer.compiles_pending() {
                self.renderer.compile_progress().await;
            }
            self.redraw_pending = true;
            self.frame()?;
            if !self.renderer.frame_complete() && !self.renderer.compiles_pending() {
                // Nothing is compiling, yet a series was left out: no
                // compile will ever wake this wait.
                return Err(JsValue::from_str(
                    "a series waits for a pipeline that no compile is pending for",
                ));
            }
        }
        Ok(())
    }

    /// Resolves once a frame draws every series of the chart (#299),
    /// drawing again as the pipelines it waits for compile. It rejects with
    /// an `Error` carrying `label`, `kind` and `message` when one of those
    /// compiles failed (see [`Self::pipeline_failures`]); the chart still
    /// draws that series, and the draw reports the error as usual.
    ///
    /// JS: `await chart.series_ready();`
    pub async fn series_ready(&mut self) -> Result<(), JsValue> {
        self.draw_until_complete().await?;
        self.renderer
            .series_ready()
            .await
            .map_err(|failure| failure_error(&failure))
    }

    /// Resolves once every background compile scheduled so far has ended
    /// (#299). It rejects like [`Self::series_ready`] with the first that
    /// failed. It draws nothing.
    ///
    /// JS: `await chart.pipelines_ready();`
    pub async fn pipelines_ready(&mut self) -> Result<(), JsValue> {
        self.renderer
            .pipelines_ready()
            .await
            .map_err(|failure| failure_error(&failure))
    }

    /// Whether the last frame drew every series of the chart (#299): false
    /// while a series waits for its pipelines' background compile, which
    /// the frames that follow draw as they resolve.
    pub fn frame_complete(&self) -> bool {
        self.renderer.frame_complete()
    }

    /// The chart's background pipeline compiles (#299) as an object:
    /// `{ screen: { total, ready, compiling, failed }, all: { … },
    /// frame_complete }`. `screen` counts what the last frame needed, so a
    /// progress bar over it never goes back; `all` grows when a chart first
    /// needs a module compiled on demand. `compiling` counts the waiting
    /// compiles too.
    ///
    /// JS: `const { screen, all, frame_complete } = chart.pipeline_status();`
    #[wasm_bindgen(unchecked_return_type = "FiggyPipelineStatus")]
    pub fn pipeline_status(&self) -> Result<JsValue, JsValue> {
        let status = self.renderer.pipeline_status();
        let object = js_sys::Object::new();
        js_sys::Reflect::set(&object, &"screen".into(), &counts_object(status.screen)?)?;
        js_sys::Reflect::set(&object, &"all".into(), &counts_object(status.all)?)?;
        js_sys::Reflect::set(
            &object,
            &"frame_complete".into(),
            &status.frame_complete.into(),
        )?;
        Ok(object.into())
    }

    /// Every background compile that failed (#299), in order, as
    /// `[{ label, kind, message }]`. `kind` is `"validation"` or
    /// `"internal"` (the browser's `GPUPipelineError` reason), `"rejected"`
    /// (any other rejection), `"device_lost"` or `"cancelled"`.
    ///
    /// JS: `for (const { label, kind } of chart.pipeline_failures()) …`
    ///
    /// Returns an array of plain objects, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "FiggyPipelineFailure[]")]
    pub fn pipeline_failures(&self) -> Result<js_sys::Array, JsValue> {
        let array = js_sys::Array::new();
        for failure in self.renderer.pipeline_failures() {
            array.push(&failure_object(&failure)?);
        }
        Ok(array)
    }

    /// A handle on the chart's background pipeline compiles that does not
    /// borrow the chart (#315): see [`FiggyReadiness`]. Its waits leave the
    /// chart free for `pipeline_status()`, `frame()`, `resize()` and
    /// `free()`; once the chart is freed they reject with a
    /// `"chart disposed"` `Error`. `series_ready()`, `pipelines_ready()` and
    /// `first_frame_ready()` on the chart keep working as before.
    ///
    /// JS: `const readiness = chart.readiness(); await readiness.pipelines_ready();`
    pub fn readiness(&self) -> FiggyReadiness {
        FiggyReadiness {
            readiness: self.renderer.pipeline_readiness(),
        }
    }

    /// Prepare the resident Cartesian pipeline groups while reporting the
    /// current stage as `{ scope, stage, phase }` objects. Separate bounded
    /// renderers, nonresident matrix/tile pipelines and future chart-specific
    /// allocations are outside this scope. The chart is mutably
    /// borrowed until the Promise settles; hosts must keep their normal
    /// async busy gate active and queue chart work rather than calling the
    /// kernel from the callback. An exception the callback throws rejects
    /// the returned promise with it once the prewarm returns; later stages
    /// are not reported.
    pub async fn prewarm_all_with_progress(
        &mut self,
        on_event: js_sys::Function,
    ) -> Result<(), JsValue> {
        let mut failed = None;
        let mut observer = |event| observe_init_progress(Some(&on_event), event, &mut failed);
        let prewarmed = self
            .renderer
            .prewarm_all_observed(&mut observer)
            .await
            .map_err(js_err);
        if let Some(error) = failed {
            return Err(error);
        }
        prewarmed?;
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
    ///
    /// A layout waiting for its fit (`layout_state: "needs_fit"` in
    /// `get_config`) is fitted first and the fit is stored (#357):
    /// `get_config` then reads `"fitted"` with the fitted margins. A chart
    /// shown from a stream, and a monotonic chart, are drawn as stored.
    /// `frame_as_stored` draws without fitting.
    pub fn frame(&mut self) -> Result<(), JsValue> {
        self.frame_with(renderer::DrawLayout::FitPending)
    }

    /// `frame`, drawing the document exactly as stored (#357): a layout
    /// waiting for its fit is drawn with the margins it holds and stays
    /// `"needs_fit"`.
    pub fn frame_as_stored(&mut self) -> Result<(), JsValue> {
        self.frame_with(renderer::DrawLayout::AsStored)
    }
}

impl FiggyChart {
    fn frame_with(&mut self, layout: renderer::DrawLayout) -> Result<(), JsValue> {
        self.renderer
            .sync_external_invalidations()
            .map_err(js_err)?;
        if layout == renderer::DrawLayout::FitPending && self.monotonic.is_none() {
            self.renderer
                .fit_pending_layout_for_draw(self.chart_id)
                .map_err(js_err)?;
        }
        let current_stamp = self.render_stamp()?;
        let renderer_dirty = current_stamp.needs_draw_since(self.last_presented_stamp.as_ref());
        let raster_dirty = current_stamp.needs_raster_since(self.last_presented_stamp.as_ref());
        // The annotation layer has its own raster revision: an item edit
        // or a changed annotation selection redraws the two annotation
        // textures only; `refresh_axis` redoes them with the axis raster.
        let annotation_dirty = current_stamp
            .needs_annotation_raster_since(self.last_presented_stamp.as_ref())
            || self.annotation_selection_dirty;
        // A pipeline a left-out series waited for has compiled: draw again,
        // so the chart fills in without a host call (#299).
        let decision = frame_decision(
            renderer_dirty,
            raster_dirty,
            self.view_dirty,
            self.redraw_pending || self.renderer.compile_redraw_due(),
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
        let current_stamp = self.render_stamp()?;

        // Browser resize is preview zoom, not a document mutation. The
        // stored chart remains the export SSoT; the live canvas renders a
        // scaled/letterboxed display chart derived from it.
        let (display_config, panel_rect, _) = self.display_config()?;
        let display_chart = Chart::new(display_config.into_owned());
        // `draw_registered` redraws the text and axis raster itself when the
        // chart, its selection or its display placement changed (#265).
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

        // The renderer draws the chart at the display placement set by
        // `set_chart_display`, skipping a series whose source is not
        // resident (#160).
        match self.monotonic {
            Some(chart) => self.renderer.draw_monotonic(
                self.clear_color,
                &mut [renderer::MonotonicChartDrawItem {
                    chart,
                    view: &mut self.view,
                }],
            ),
            // The layout was settled above: the draw shows the chart as
            // stored either way.
            None => self.renderer.draw_registered_as_stored(
                self.clear_color,
                &mut [RegisteredChartDrawItem {
                    chart_id: self.chart_id,
                    view: &mut self.view,
                }],
            ),
        }
        .map_err(js_err)?;

        // Only a successfully submitted/presented draw advances the
        // renderer-issued onscreen stamp. Any earlier failure leaves the
        // frame dirty so the next persistent rAF retries it.
        consume_successful_frame(&mut self.view_dirty, &mut self.redraw_pending);
        self.annotation_selection_dirty = false;
        self.last_presented_stamp = Some(current_stamp);
        Ok(())
    }
}

#[wasm_bindgen]
impl FiggyChart {
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
        // A monotonic chart resizes through its resize call (#271): with no
        // sources it is refused when a group's mapping would move
        // (`resize_monotonic` takes them).
        if self.monotonic.is_some() {
            return self.resize_monotonic(w, h, js_sys::Array::new(), JsValue::UNDEFINED);
        }
        self.renderer
            .set_chart_display(self.chart_id, Some((w, h)))
            .map_err(js_err)?;
        self.renderer.resize(w, h).map_err(js_err)?;
        self.surface_size = (w, h);
        self.view_dirty = true;
        Ok(())
    }
}
