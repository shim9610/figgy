//! `FiggyChart` construction, shader sets, fonts and GPU memory.

use super::*;

#[wasm_bindgen]
impl FiggyChart {
    /// Bind the low-level chart kernel to `canvas` (uses the canvas's
    /// current pixel size). Ordinary web hosts should prefer
    /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
    /// JS: `const chart = await FiggyChart.create(canvas);`
    pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, None, renderer::ShaderSet::Basic).await
    }

    /// Same as [`Self::create`], reporting each init stage to `on_event`
    /// as `{ scope, stage, phase }`. Do not call other kernel methods
    /// from the callback. An exception the callback throws rejects the
    /// returned promise with it; later stages are not reported.
    pub async fn create_with_progress(
        canvas: HtmlCanvasElement,
        on_event: js_sys::Function,
    ) -> Result<FiggyChart, JsValue> {
        create_chart_kernel(canvas, Some(on_event), renderer::ShaderSet::Basic).await
    }

    /// [`Self::create`] with options (#299), all optional:
    ///
    /// - `shaders`: `"basic"` (the default) or `"full"`. A chart starts on
    ///   the basic shader set, which draws linear and logarithmic
    ///   rectangular charts, and moves to the full set by itself when its
    ///   config first has an axis break, a probability axis or polar
    ///   coordinates. A host that knows it will show such a chart passes
    ///   `"full"`, so the basic set is never compiled and discarded.
    /// - `on_progress`: the callback of [`Self::create_with_progress`].
    ///
    /// An unknown option, or a `shaders` value other than those two,
    /// rejects the promise with an `Error` that names it; nothing is
    /// created.
    /// JS: `const chart = await FiggyChart.create_with_options(canvas, { shaders: "full" });`
    pub async fn create_with_options(
        canvas: HtmlCanvasElement,
        options: JsValue,
    ) -> Result<FiggyChart, JsValue> {
        let (set, on_progress) = create_options(&options)?;
        create_chart_kernel(canvas, on_progress, set).await
    }

    /// The shader set this chart compiles (#299): `"basic"` or `"full"`.
    ///
    /// Returns a plain string, not JSON.
    #[wasm_bindgen(unchecked_return_type = "\"basic\" | \"full\"")]
    pub fn shader_set(&self) -> String {
        shader_set_name(self.renderer.shader_set()).into()
    }

    /// Move this chart to at least the shader set `shaders` (`"basic"` or
    /// `"full"`, #299), as its first chart with an axis break, a
    /// probability axis or polar coordinates would. A chart never goes
    /// back, so `"basic"` and a repeated `"full"` do nothing. Any other
    /// value throws an `Error` that names it.
    pub fn require_shader_set(&mut self, shaders: JsValue) -> Result<(), JsValue> {
        let set = shader_set_from_name(&shaders)?;
        self.renderer.require_shader_set(set);
        Ok(())
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

    /// Pool growth on an otherwise out-of-space upload, on by default.
    /// Growth remains bounded by the device storage-binding limit and
    /// the configured renderer memory budget; `false` keeps the pool's
    /// current capacity.
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Sizes are in bytes.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn gpu_memory_status(&self) -> String {
        let usage = self.renderer.gpu_memory_usage();
        let pool = self.renderer.pool();
        let mut sources: Vec<_> = self
            .renderer
            .logical_column_ids()
            .filter_map(|id| {
                let column = self.renderer.logical_column(id)?;
                let (residency, encoded_bytes) = match column {
                    renderer::LogicalColumn::Resident(slot) => ("resident", Some(slot.byte_size)),
                    renderer::LogicalColumn::Streamed(_) => ("streamed", None),
                };
                Some(serde_json::json!({
                    "id": id,
                    "length": column.len(),
                    "revision": self.renderer.column_revision(id).map(|r| r.to_string()),
                    "residency": residency,
                    "encoded_bytes": encoded_bytes,
                }))
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
                "retired_bytes": usage.retired_bytes_of(renderer::GpuResourceKind::ColumnPool),
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
    ///
    /// Resolves to JSON text (`FiggyJson`) in the shape of `gpu_memory_status`,
    /// not an object.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
}
