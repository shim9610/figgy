//! figgy for the web — a `wasm-bindgen` wrapper exposing one chart panel per
//! `<canvas>` as the `FiggyChart` JS class.
//!
//! Lifecycle model: **one instance, register/unregister**. Columns and series
//! are managed by id — `set_column_f32` is an upsert that skips re-uploading
//! identical data, `remove_*` unregisters (and drops dependents), and pool
//! defragmentation runs automatically after removals. Pool internals
//! (capacity stats, defrag policy) are intentionally not exposed.
//!
//! The whole implementation is gated to `wasm32`: on native targets this
//! crate compiles to an empty library so `cargo test --workspace` stays
//! green. Build the artifact with:
//!
//! ```bash
//! npx wasm-pack build crates/web --release --target web
//! ```
//!
//! See `crates/renderer/WASM.md` for the I/O architecture this implements.

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;

    use wasm_bindgen::prelude::*;
    use web_sys::HtmlCanvasElement;

    use renderer::data::Column;
    use renderer::data_config::ErrorRef;
    use renderer::layout::{ChartArea, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::rich_segments_from_text;
    use renderer::{
        Chart, ChartDrawItem, ChartStyle, ChartView, Color, CpuTextMeasure, DataLineStyleConfig,
        DataRenderType, DefragPolicy, HitId, HitMap, Renderer,
        ResizeHandle as ModelResizeHandle, SelectionBox, Series, SeriesConfig, WindowedRenderer,
    };

    const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

    fn js_err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }

    /// 64-bit FNV-1a-style mix over the f32 bit patterns — cheap identity
    /// check so `set_column_f32` can skip re-uploading unchanged data.
    fn hash_f32s(data: &[f32]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for v in data {
            h ^= v.to_bits() as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// Every column id a series' render type references (x/y + error refs).
    fn referenced_columns(cfg: &SeriesConfig) -> Vec<&str> {
        fn push_ref<'a>(ids: &mut Vec<&'a str>, r: &'a ErrorRef) {
            match r {
                ErrorRef::Symmetric { column } => ids.push(column),
                ErrorRef::Asymmetric { lower, upper } => {
                    ids.push(lower);
                    ids.push(upper);
                }
            }
        }

        let mut ids: Vec<&str> = vec![&cfg.x_column, &cfg.y_column];
        match &cfg.render_type {
            DataRenderType::Scatter { .. }
            | DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. } => {}
            DataRenderType::ScatterErrorbarX { err_x, .. }
            | DataRenderType::LineScatterErrorbarX { err_x, .. } => push_ref(&mut ids, err_x),
            DataRenderType::ScatterErrorbarY { err_y, .. }
            | DataRenderType::LineScatterErrorbarY { err_y, .. } => push_ref(&mut ids, err_y),
            DataRenderType::ScatterErrorbarXY { err_x, err_y, .. }
            | DataRenderType::LineScatterErrorbarXY { err_x, err_y, .. } => {
                push_ref(&mut ids, err_x);
                push_ref(&mut ids, err_y);
            }
        }
        ids
    }

    // ------------------------------------------------------------------
    // Preset mirrors — fieldless enums cross the boundary as integers.
    // ------------------------------------------------------------------

    /// Axis frame presets (mirror of `model::AxisPreset`).
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum AxisPreset {
        BoxedInward,
        BoxedOutward,
        OpenOutward,
        OpenInward,
        Minimal,
    }

    impl From<AxisPreset> for renderer::AxisPreset {
        fn from(p: AxisPreset) -> Self {
            match p {
                AxisPreset::BoxedInward => Self::BoxedInward,
                AxisPreset::BoxedOutward => Self::BoxedOutward,
                AxisPreset::OpenOutward => Self::OpenOutward,
                AxisPreset::OpenInward => Self::OpenInward,
                AxisPreset::Minimal => Self::Minimal,
            }
        }
    }

    /// Series color rotations (mirror of `model::ColorCycle`).
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum ColorCycle {
        Classic,
        Vivid,
        Balanced,
        ColorblindSafe,
        Monochrome,
    }

    impl From<ColorCycle> for renderer::ColorCycle {
        fn from(c: ColorCycle) -> Self {
            match c {
                ColorCycle::Classic => Self::Classic,
                ColorCycle::Vivid => Self::Vivid,
                ColorCycle::Balanced => Self::Balanced,
                ColorCycle::ColorblindSafe => Self::ColorblindSafe,
                ColorCycle::Monochrome => Self::Monochrome,
            }
        }
    }

    /// CSS color strings (`"rgb(r g b / a)"`) for a cycle — lets the host UI
    /// render swatches / legends with exactly the chart's palette.
    #[wasm_bindgen]
    pub fn color_cycle_css(cycle: ColorCycle) -> Vec<String> {
        renderer::ColorCycle::from(cycle)
            .colors()
            .iter()
            .map(|c| {
                format!(
                    "rgb({} {} {} / {})",
                    (c.r * 255.0).round() as u8,
                    (c.g * 255.0).round() as u8,
                    (c.b * 255.0).round() as u8,
                    c.a,
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // FiggyChart — one panel bound to one canvas.
    // ------------------------------------------------------------------

    #[wasm_bindgen]
    pub struct FiggyChart {
        renderer: WindowedRenderer<'static>,
        chart: Chart,
        view: ChartView,
        series_cfgs: Vec<SeriesConfig>,
        styles: Vec<ChartStyle>,
        /// 1:1 with `series_cfgs` — legend label text (None = no legend row).
        labels: Vec<Option<String>>,
        /// Registered columns: id → (len, content hash) for upsert skipping.
        columns: HashMap<String, (usize, u64)>,
        /// A removal happened — defragment once on the next frame.
        needs_defrag: bool,
        /// Monotonic color assignment for newly registered series.
        color_seq: usize,
        hitmap: HitMap,
        selected: Option<HitId>,
        dragging: bool,
        resizing: Option<ModelResizeHandle>,
        cycle: renderer::ColorCycle,
    }

    impl FiggyChart {
        /// Rebuild the legend from the series registry — the legend always
        /// mirrors the registered series (one line per labeled series,
        /// composed into the legend's one-document content with explicit
        /// `'\n'` segments between entries). The symbol is derived from each
        /// series' render type (`—`, `●`, …) as inline segments carrying the
        /// series color override; labels keep `'\n'` line breaks and the
        /// unicode sub/superscript mapping. The user's legend font,
        /// font_size, and color survive the rebuild — only the segments are
        /// recomposed.
        fn rebuild_legend(&mut self) {
            let entries: Vec<(SeriesConfig, String)> = self
                .series_cfgs
                .iter()
                .zip(self.labels.iter())
                .filter_map(|(cfg, label)| Some((cfg.clone(), label.clone()?)))
                .collect();

            self.chart.with_decoration_change(|c| {
                let mut content = c.legend.content.clone();
                content.segments.clear();
                for (cfg, label) in &entries {
                    renderer::config::append_legend_entry(
                        &mut content,
                        renderer::config::series_symbol_segments(cfg),
                        label,
                    );
                }
                c.legend.visible = !content.segments.is_empty();
                c.legend.content = content;
            });
        }

        fn rebuild_styles(&mut self) {
            self.styles = self
                .series_cfgs
                .iter()
                .map(|cfg| self.renderer.create_style_for_series(cfg))
                .collect();
        }

        fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
            for id in referenced_columns(cfg) {
                if !self.columns.contains_key(id) {
                    return Err(js_err(format!(
                        "series '{}' references unregistered column '{id}'",
                        cfg.series_id
                    )));
                }
            }
            Ok(())
        }

        /// Errorbar variants bind the `"__zero"` column for their unused error
        /// dimension. Provision it transparently (sized to the longest column
        /// any errorbar series references) so hosts never learn the
        /// convention. Upsert semantics make repeat calls free.
        fn ensure_zero_column(&mut self) -> Result<(), JsValue> {
            let mut needed = 0usize;
            for cfg in &self.series_cfgs {
                let is_plain = matches!(
                    cfg.render_type,
                    DataRenderType::Line { .. }
                        | DataRenderType::Scatter { .. }
                        | DataRenderType::ScatterLine { .. }
                );
                if !is_plain {
                    for id in referenced_columns(cfg) {
                        if let Some(&(len, _)) = self.columns.get(id) {
                            needed = needed.max(len);
                        }
                    }
                }
            }
            let existing = self.columns.get("__zero").map(|&(len, _)| len).unwrap_or(0);
            if needed > 0 && existing < needed {
                self.set_column_f32("__zero", &vec![0.0; needed])?;
            }
            Ok(())
        }
    }

    #[wasm_bindgen]
    impl FiggyChart {
        /// Bind a chart to `canvas` (uses the canvas's current pixel size).
        /// JS: `const chart = await FiggyChart.create(canvas);`
        pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
            console_error_panic_hook::set_once();

            let (w, h) = (canvas.width().max(1), canvas.height().max(1));
            let mut renderer = Renderer::for_window_async(
                wgpu::SurfaceTarget::Canvas(canvas),
                (w, h),
                POOL_CAPACITY,
            )
            .await
            .map_err(js_err)?;
            // Replace-heavy hosts can hit transient fragmentation between the
            // remove and the next frame's defrag — let the pool self-heal.
            renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);

            let mut config = renderer::default::default_config();
            config.chart_area = ChartArea(Rect { x: 0, y: 0, width: w, height: h });
            let chart = Chart::new(config);
            let view = renderer
                .create_chart_view(&chart, Rect { x: 0, y: 0, width: w, height: h })
                .map_err(js_err)?;

            Ok(FiggyChart {
                renderer,
                chart,
                view,
                series_cfgs: Vec::new(),
                styles: Vec::new(),
                labels: Vec::new(),
                columns: HashMap::new(),
                needs_defrag: false,
                color_seq: 0,
                hitmap: HitMap::standard_chart(),
                selected: None,
                dragging: false,
                resizing: None,
                cycle: renderer::ColorCycle::Classic,
            })
        }

        /// Register a font (TTF/OTF/TTC bytes) for SSoT `font` family names.
        /// Returns the family names the file declares — use them verbatim in
        /// `content.font` / label styles. Registered fonts win over native
        /// system fonts, so resolution behaves identically on web and
        /// desktop. Already-drawn text re-rasterizes on the next `frame()`.
        ///
        /// JS: `chart.register_font(new Uint8Array(await (await fetch(url)).arrayBuffer()))`
        pub fn register_font(&mut self, bytes: &[u8]) -> Result<Vec<String>, JsValue> {
            let families =
                renderer::text_render::register_font_bytes(bytes.to_vec()).map_err(js_err)?;
            // Text may already be on screen in the fallback font — force a
            // decoration re-raster so the registration is visible.
            self.chart.with_decoration_change(|_| {});
            Ok(families)
        }

        // ---- column registry (id-keyed upsert / unregister) ----

        /// Register or update a data column under `id` (`Float32Array`).
        ///
        /// Upsert semantics, fully automatic:
        /// - new id → upload;
        /// - same id + **identical data** (length + content hash) → no-op,
        ///   the existing GPU mapping is kept — callers can stream their whole
        ///   dataset every time without redundant uploads;
        /// - same id + different data → replace (the old region is freed and
        ///   the pool defragments on the next frame).
        pub fn set_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
            let signature = (data.len(), hash_f32s(data));
            if self.columns.get(id) == Some(&signature) {
                return Ok(());
            }
            if self.columns.contains_key(id) {
                self.renderer.remove_column(id);
                self.needs_defrag = true;
            }

            let (mut min, mut max) = (f32::INFINITY, f32::NEG_INFINITY);
            for &v in data {
                if v < min { min = v; }
                if v > max { max = v; }
            }
            let column = Column { data: data.to_vec(), min, max };
            self.renderer.add_column(id, &column).map_err(js_err)?;
            self.columns.insert(id.to_string(), signature);
            Ok(())
        }

        /// Unregister a column. Series referencing it are removed too (with
        /// their legend rows) so the chart can never point at freed data.
        /// Returns `true` when the column existed.
        pub fn remove_column(&mut self, id: &str) -> bool {
            if self.columns.remove(id).is_none() {
                return false;
            }
            self.renderer.remove_column(id);
            self.needs_defrag = true;

            let keep: Vec<bool> = self
                .series_cfgs
                .iter()
                .map(|cfg| !referenced_columns(cfg).contains(&id))
                .collect();
            if keep.iter().any(|k| !k) {
                let mut it = keep.iter();
                self.series_cfgs.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.styles.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.labels.retain(|_| *it.next().unwrap());
                self.rebuild_legend();
            }
            true
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
            let existing = self.series_cfgs.iter().position(|c| c.series_id == series_id);
            let color = match existing {
                Some(i) => match &self.series_cfgs[i].render_type {
                    DataRenderType::Line { line } => line.line_color,
                    _ => self.cycle.color(i),
                },
                None => {
                    let c = self.cycle.color(self.color_seq);
                    self.color_seq += 1;
                    c
                }
            };
            let cfg = SeriesConfig {
                series_id: series_id.into(),
                label: None,
                x_column: x_column.into(),
                y_column: y_column.into(),
                render_type: DataRenderType::Line {
                    line: DataLineStyleConfig {
                        line_style: LineStylePreset::Solid,
                        line_color: color,
                        line_width: line_width.max(0.5),
                    },
                },
            };
            self.ensure_columns_exist(&cfg)?;
            let style = self.renderer.create_style_for_series(&cfg);
            let label = (!label.is_empty()).then(|| label.to_string());

            match existing {
                Some(i) => {
                    self.series_cfgs[i] = cfg;
                    self.styles[i] = style;
                    self.labels[i] = label;
                }
                None => {
                    self.series_cfgs.push(cfg);
                    self.styles.push(style);
                    self.labels.push(label);
                }
            }
            self.rebuild_legend();
            Ok(())
        }

        /// Set / change / remove a series' legend label. `'\n'` breaks lines;
        /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments.
        /// Empty string removes the legend row. Returns `true` when the
        /// series exists.
        pub fn set_series_label(&mut self, series_id: &str, label: &str) -> bool {
            let Some(i) = self.series_cfgs.iter().position(|c| c.series_id == series_id)
            else {
                return false;
            };
            self.labels[i] = (!label.is_empty()).then(|| label.to_string());
            self.rebuild_legend();
            true
        }

        /// Unregister a series (and its legend row). Columns stay registered.
        /// Returns `true` when the series existed.
        pub fn remove_series(&mut self, series_id: &str) -> bool {
            let Some(i) = self.series_cfgs.iter().position(|c| c.series_id == series_id)
            else {
                return false;
            };
            self.series_cfgs.remove(i);
            self.styles.remove(i);
            self.labels.remove(i);
            self.rebuild_legend();
            true
        }

        /// Fit the x axis to a column's range with proportional padding.
        pub fn auto_fit_x(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            self.chart.auto_fit_x(self.renderer.pool(), column, padding).map_err(js_err)
        }

        pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            self.chart.auto_fit_y(self.renderer.pool(), column, padding).map_err(js_err)
        }

        /// Fit BOTH axes to the union of every registered series, leaving a
        /// uniform `padding` fraction of the data span as margin on each
        /// side (`0.0` = exact fit, `0.05` = 5% top/bottom/left/right).
        /// This is the whole fit policy — no rounding of the range ends;
        /// ticks land on nice values inside the range by themselves. Hosts
        /// should call this instead of re-deriving ranges.
        pub fn auto_fit_all(&mut self, padding: f64) -> Result<(), JsValue> {
            if self.series_cfgs.is_empty() {
                return Ok(());
            }
            let xs: Vec<&str> =
                self.series_cfgs.iter().map(|c| c.x_column.as_str()).collect();
            let ys: Vec<&str> =
                self.series_cfgs.iter().map(|c| c.y_column.as_str()).collect();
            self.chart
                .auto_fit_x_union(self.renderer.pool(), &xs, padding)
                .map_err(js_err)?;
            self.chart
                .auto_fit_y_union(self.renderer.pool(), &ys, padding)
                .map_err(js_err)
        }

        // ---- titles ----

        pub fn set_title(&mut self, text: &str) {
            self.chart.with_decoration_change(|c| {
                c.chart_title.text.segments = rich_segments_from_text(text);
            });
        }

        pub fn set_x_title(&mut self, text: &str) {
            self.chart.with_decoration_change(|c| {
                c.bottom_x.title_option.text.segments = rich_segments_from_text(text);
            });
        }

        pub fn set_y_title(&mut self, text: &str) {
            self.chart.with_decoration_change(|c| {
                c.left_y.title_option.text.segments = rich_segments_from_text(text);
            });
        }

        // ---- presets ----

        /// Apply an axis frame preset to all four axes (decoration-only).
        pub fn apply_axis_preset(&mut self, preset: AxisPreset) {
            let p: renderer::AxisPreset = preset.into();
            self.chart.with_decoration_change(|c| c.apply_axis_preset(p));
        }

        /// Switch the series color rotation: recolors every series in order,
        /// rebuilds their GPU styles, and keeps legend swatches in sync.
        pub fn apply_color_cycle(&mut self, cycle: ColorCycle) {
            self.cycle = cycle.into();
            for (i, cfg) in self.series_cfgs.iter_mut().enumerate() {
                self.cycle.apply_to_series(cfg, i);
            }
            self.color_seq = self.series_cfgs.len();
            self.rebuild_styles();
            self.rebuild_legend();
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
            serde_json::to_string(self.chart.config()).map_err(js_err)
        }

        /// Replace the whole option SSoT from JSON. Marks everything dirty —
        /// the next `frame()` re-rasters the chrome and refreshes the
        /// transform, exactly like any other config edit.
        /// JS: `chart.set_config(JSON.stringify(cfg));`
        pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
            let new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
            *self.chart.config_mut() = new_cfg;
            Ok(())
        }

        /// Serialize the series declarations (columns, render type, styles).
        pub fn get_series(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.series_cfgs).map_err(js_err)
        }

        /// Replace the series declarations from JSON. Column references must
        /// already be registered; GPU styles are rebuilt, legend labels are
        /// kept for series ids that survive.
        pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
            let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
            for cfg in &new_series {
                self.ensure_columns_exist(cfg)?;
            }
            self.labels = new_series
                .iter()
                .map(|cfg| {
                    self.series_cfgs
                        .iter()
                        .position(|old| old.series_id == cfg.series_id)
                        .and_then(|i| self.labels[i].clone())
                })
                .collect();
            self.series_cfgs = new_series;
            self.color_seq = self.series_cfgs.len().max(self.color_seq);
            self.ensure_zero_column()?;
            self.rebuild_styles();
            self.rebuild_legend();
            Ok(())
        }

        // ---- pointer interaction (coordinates in canvas pixels) ----

        /// Hit-test the chart chrome at canvas pixel `(x, y)` — returns the
        /// topmost element's stable id (`"data_area"`, `"axis_bottom"`,
        /// `"tick_labels_left"`, `"axis_title_left"`, `"legend"`,
        /// `"chart_title"`, …) or `null`. Pure geometry, no selection state
        /// change: the renderer's own layout answers, so hosts don't have to
        /// re-derive box positions for hover cursors / context UI.
        pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
            self.hitmap
                .hit_test(self.chart.config(), &CpuTextMeasure, x, y)
                .and_then(|id| self.hitmap.get(id))
                .map(|el| el.element_id())
        }

        /// Pointer press. Returns `true` while something is selected — the
        /// host can mirror that state in its own UI.
        pub fn on_press(&mut self, x: f32, y: f32) -> bool {
            // Resize handles on the selected element win over hit-testing.
            if let Some(id) = self.selected
                && let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                && let Some(handle) =
                    rz.hit_resize_handle(self.chart.config(), &CpuTextMeasure, x, y)
            {
                self.resizing = Some(handle);
                self.dragging = false;
                return true;
            }
            self.resizing = None;

            let new_sel = self.hitmap.hit_test(self.chart.config(), &CpuTextMeasure, x, y);
            self.dragging = new_sel.is_some_and(|id| {
                self.hitmap.get(id).is_some_and(|el| el.as_draggable().is_some())
            });
            if new_sel != self.selected {
                self.selected = new_sel;
                self.chart.with_decoration_change(|_| {});
            }
            self.selected.is_some()
        }

        /// Pointer move with frame delta — drags or resizes the selection.
        pub fn on_move(&mut self, dx: f32, dy: f32) {
            let Some(id) = self.selected else { return };
            if let Some(handle) = self.resizing {
                if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable()) {
                    let _ = rz.resize_by(self.chart.config_mut(), handle, dx, dy);
                }
                return;
            }
            if self.dragging
                && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
            {
                let _ = drag.drag_by(self.chart.config_mut(), dx, dy);
            }
        }

        pub fn on_release(&mut self) {
            self.dragging = false;
            self.resizing = None;
        }

        pub fn has_selection(&self) -> bool {
            self.selected.is_some()
        }

        // ---- frame / resize / export ----

        /// Process pending pool maintenance + dirty flags, then draw one
        /// frame. Call from `requestAnimationFrame`; with nothing dirty the
        /// raster cost is skipped and only the GPU pass runs.
        pub fn frame(&mut self) -> Result<(), JsValue> {
            // Coalesced auto-defrag: removals since the last frame collapse
            // into one compaction pass (GPU-internal copies only).
            if self.needs_defrag {
                self.needs_defrag = false;
                self.renderer.defragment().map_err(js_err)?;
            }

            // chart_area is the panel-rect SSoT — set_config / resize may
            // have moved it, so derive the rect from the config, not the view.
            let panel_rect = self.chart.config().chart_area.0;
            if self.chart.consume_raster_dirty() {
                let sel_boxes: Vec<SelectionBox> = self
                    .selected
                    .and_then(|id| {
                        self.hitmap.selection_box(id, self.chart.config(), &CpuTextMeasure)
                    })
                    .into_iter()
                    .collect();
                self.renderer
                    .refresh_axis_with_selection(
                        &mut self.view,
                        &self.chart,
                        panel_rect,
                        &sel_boxes,
                    )
                    .map_err(js_err)?;
                let _ = self.chart.consume_data_dirty();
            } else if self.chart.consume_data_dirty() {
                self.renderer.update_transform(&self.view, &self.chart);
            }

            let series: Vec<Series<'_>> = self
                .series_cfgs
                .iter()
                .zip(self.styles.iter())
                .map(|(cfg, style)| Series { config: cfg, style })
                .collect();
            let items = [ChartDrawItem {
                view: &self.view,
                chart_config: self.chart.config(),
                series: &series,
            }];
            self.renderer.draw(Color::WHITE, &items).map_err(js_err)
        }

        /// Resize the swap chain + chart area to new canvas pixel dimensions.
        pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
            let (w, h) = (width.max(1), height.max(1));
            self.renderer.resize(w, h).map_err(js_err)?;
            self.chart.config_mut().chart_area =
                ChartArea(Rect { x: 0, y: 0, width: w, height: h });
            Ok(())
        }

        /// Export the panel as PNG bytes at `scale ×` resolution.
        /// JS: `const png = await chart.export_png(2.0);`
        /// (`&mut self`: the renderer's export runs an arc-prefix prepare
        /// phase; wasm-bindgen serializes access, so this changes nothing
        /// for JS callers.)
        pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
            let bytes = self
                .renderer
                .export_panel_png_bytes_async(&self.chart, &self.series_cfgs, scale)
                .await
                .map_err(js_err)?;
            Ok(js_sys::Uint8Array::from(bytes.as_slice()))
        }

        /// Load the bundled demo (sine + RC charge curves) — lets a frontend
        /// see a real chart without wiring data first. Idempotent: columns
        /// and series are upserts, so calling it twice changes nothing.
        pub fn load_demo(&mut self) -> Result<(), JsValue> {
            let (xs, ys) = renderer::demo::sine_data(512);
            let (ts, vs) = renderer::demo::rc_data(512);
            let to_f32 = |v: Vec<f64>| v.into_iter().map(|x| x as f32).collect::<Vec<f32>>();
            let (xs, ys, ts, vs) = (to_f32(xs), to_f32(ys), to_f32(ts), to_f32(vs));

            self.set_column_f32("demo_x", &xs)?;
            self.set_column_f32("demo_sin", &ys)?;
            self.set_column_f32("demo_t", &ts)?;
            self.set_column_f32("demo_rc", &vs)?;
            self.add_line_series("sine", "demo_x", "demo_sin", 2.0, "sin(x)")?;
            self.add_line_series("rc", "demo_t", "demo_rc", 2.0, "RC charge")?;
            self.auto_fit_x("demo_x", 0.02)?;
            self.auto_fit_y("demo_sin", 0.10)?;
            self.set_title("figgy");
            self.set_x_title("x");
            self.set_y_title("y");
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
