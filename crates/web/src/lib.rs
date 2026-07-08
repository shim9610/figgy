//! figgy for the web.
//!
//! The public browser surface is the `figgy-chart.js` Custom Element
//! (`<figgy-chart>`). It owns the canvas, async initialization, rAF loop,
//! resize/DPR handling, pointer wiring, busy gate, and CustomEvent output.
//! This Rust module exposes the raw `FiggyChart` wasm class that the Custom
//! Element uses as its low-level kernel; advanced hosts may call it directly
//! when they intentionally want to own those browser responsibilities.
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

#[cfg(any(target_arch = "wasm32", test))]
fn fit_display_panel(
    logical_size: (u32, u32),
    surface_size: (u32, u32),
) -> (f32, renderer::layout::Rect) {
    let doc_w = logical_size.0.max(1) as f32;
    let doc_h = logical_size.1.max(1) as f32;
    let surface_w = surface_size.0.max(1);
    let surface_h = surface_size.1.max(1);
    let scale = ((surface_w as f32) / doc_w).min((surface_h as f32) / doc_h);
    let panel_w = ((doc_w * scale).round().max(1.0) as u32).min(surface_w);
    let panel_h = ((doc_h * scale).round().max(1.0) as u32).min(surface_h);
    let x = (surface_w - panel_w) / 2;
    let y = (surface_h - panel_h) / 2;
    (
        scale,
        renderer::layout::Rect {
            x,
            y,
            width: panel_w,
            height: panel_h,
        },
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn display_config_for_surface(
    config: &renderer::Config,
    surface_size: (u32, u32),
) -> (renderer::Config, renderer::layout::Rect, f32) {
    let logical = config.chart_area.0;
    let (scale, panel_rect) = fit_display_panel((logical.width, logical.height), surface_size);
    let mut display_config = config.scaled(scale);
    display_config.chart_area = renderer::layout::ChartArea(panel_rect);
    (display_config, panel_rect, scale)
}

#[cfg(any(target_arch = "wasm32", test))]
fn picked_point_json_string(picked: &renderer::PickedPoint) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": picked.source_id.as_ref(),
        "series_id": &picked.series_id,
        "point_index": picked.point_index,
        "data_x": picked.data_x,
        "data_y": picked.data_y,
        "distance_px": picked.distance_px,
    }))
}

#[cfg(test)]
mod tests {
    use super::{display_config_for_surface, fit_display_panel, picked_point_json_string};

    #[test]
    fn display_panel_uniformly_scales_document() {
        let (scale, panel) = fit_display_panel((1000, 800), (2000, 1600));
        assert!((scale - 2.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 2000, 1600)
        );
    }

    #[test]
    fn display_panel_letterboxes_aspect_ratio_changes() {
        let (scale, panel) = fit_display_panel((1000, 800), (1600, 800));
        assert!((scale - 1.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (300, 0, 1000, 800)
        );
    }

    #[test]
    fn display_config_scales_without_mutating_logical_document() {
        let mut config = renderer::default::default_config();
        config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        });
        let original = config.clone();

        let (display, panel, scale) = display_config_for_surface(&config, (500, 400));

        assert!((scale - 0.5).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 500, 400)
        );
        assert_eq!(config.chart_area, original.chart_area);
        assert_eq!(
            config.bottom_x.label_style.font_size,
            original.bottom_x.label_style.font_size
        );
        assert_eq!(display.chart_area.0.width, 500);
        assert!((display.bottom_x.label_style.font_size - 9.0).abs() < 1e-6);
    }

    #[test]
    fn picked_point_json_includes_data_coordinates() {
        let picked = renderer::PickedPoint {
            source_id: Some("source-a".into()),
            series_id: "series-a".into(),
            point_index: 7,
            data_x: 12.5,
            data_y: -3.25,
            distance_px: 4.0,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_point_json_string(&picked).unwrap()).unwrap();

        assert_eq!(json["source_id"], "source-a");
        assert_eq!(json["series_id"], "series-a");
        assert_eq!(json["point_index"], 7);
        assert_eq!(json["data_x"], 12.5);
        assert_eq!(json["data_y"], -3.25);
        assert_eq!(json["distance_px"], 4.0);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::collections::HashMap;

    use wasm_bindgen::prelude::*;
    use web_sys::HtmlCanvasElement;

    use renderer::data::Column;
    use renderer::data_config::ErrorRef;
    use renderer::layout::{ChartArea, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::{RichText, rich_segments_from_text};
    use renderer::{
        Chart, ChartDrawItem, ChartStyle, ChartView, Color, CpuTextMeasure, DataLineStyleConfig,
        DataRenderType, DefragPolicy, FitExtent, HitId, HitMap, PointColumnLookup,
        PointPickOptions, Renderer, ResizeHandle as ModelResizeHandle, SelectionBox, Series,
        SeriesConfig, WindowedRenderer, errorbar_extent,
    };

    const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

    fn js_err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
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

        fn push_scatter_ref<'a>(
            ids: &mut Vec<&'a str>,
            scatter: &'a renderer::DataScatterStyleConfig,
        ) {
            if let Some(column) = &scatter.point_style_index_column {
                ids.push(column);
            }
        }

        fn push_errorbar_style_ref<'a>(
            ids: &mut Vec<&'a str>,
            err_style: &'a renderer::DataErrorBarStyleConfig,
        ) {
            if let Some(column) = &err_style.error_bar_style_index_column {
                ids.push(column);
            }
        }

        let mut ids: Vec<&str> = vec![&cfg.x_column, &cfg.y_column];
        match &cfg.render_type {
            DataRenderType::Scatter { scatter } => push_scatter_ref(&mut ids, scatter),
            DataRenderType::Line { .. } => {}
            DataRenderType::ScatterLine { scatter, .. } => push_scatter_ref(&mut ids, scatter),
            DataRenderType::ScatterErrorbarX {
                scatter,
                err_x,
                err_style,
            }
            | DataRenderType::LineScatterErrorbarX {
                scatter,
                err_x,
                err_style,
                ..
            } => {
                push_scatter_ref(&mut ids, scatter);
                push_errorbar_style_ref(&mut ids, err_style);
                push_ref(&mut ids, err_x);
            }
            DataRenderType::ScatterErrorbarY {
                scatter,
                err_y,
                err_style,
            }
            | DataRenderType::LineScatterErrorbarY {
                scatter,
                err_y,
                err_style,
                ..
            } => {
                push_scatter_ref(&mut ids, scatter);
                push_errorbar_style_ref(&mut ids, err_style);
                push_ref(&mut ids, err_y);
            }
            DataRenderType::ScatterErrorbarXY {
                scatter,
                err_x,
                err_y,
                err_style,
                ..
            }
            | DataRenderType::LineScatterErrorbarXY {
                scatter,
                err_x,
                err_y,
                err_style,
                ..
            } => {
                push_scatter_ref(&mut ids, scatter);
                push_errorbar_style_ref(&mut ids, err_style);
                push_ref(&mut ids, err_x);
                push_ref(&mut ids, err_y);
            }
        }
        ids
    }

    /// The (x, y) errorbar refs of a render type, when present.
    fn err_refs(rt: &DataRenderType) -> (Option<&ErrorRef>, Option<&ErrorRef>) {
        match rt {
            DataRenderType::Scatter { .. }
            | DataRenderType::Line { .. }
            | DataRenderType::ScatterLine { .. } => (None, None),
            DataRenderType::ScatterErrorbarX { err_x, .. }
            | DataRenderType::LineScatterErrorbarX { err_x, .. } => (Some(err_x), None),
            DataRenderType::ScatterErrorbarY { err_y, .. }
            | DataRenderType::LineScatterErrorbarY { err_y, .. } => (None, Some(err_y)),
            DataRenderType::ScatterErrorbarXY { err_x, err_y, .. }
            | DataRenderType::LineScatterErrorbarXY { err_x, err_y, .. } => {
                (Some(err_x), Some(err_y))
            }
        }
    }

    /// (lower, upper) error column ids of one ref — symmetric refs read the
    /// same column for both offsets, exactly like the GPU binding.
    fn err_cols(r: &ErrorRef) -> (&str, &str) {
        match r {
            ErrorRef::Symmetric { column } => (column, column),
            ErrorRef::Asymmetric { lower, upper } => (lower, upper),
        }
    }

    /// One series' errorbar fit extents + the column-content signatures they
    /// were computed from. See `FiggyChart::err_fit`.
    struct ErrFitEntry {
        /// (column id, (len, content hash)) of every involved column, in a
        /// fixed derivation order — wholesale equality is the validity test.
        sigs: Vec<(String, (usize, u64))>,
        x: Option<FitExtent>,
        y: Option<FitExtent>,
    }

    struct WebPointColumns<'a> {
        col_data: &'a HashMap<String, Vec<f32>>,
    }

    impl PointColumnLookup for WebPointColumns<'_> {
        fn get_f32_column(&self, id: &renderer::ColumnId) -> Option<&[f32]> {
            self.col_data.get(id).map(Vec::as_slice)
        }
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
    // FiggyChart — low-level wasm kernel bound to one canvas.
    // ------------------------------------------------------------------

    #[wasm_bindgen]
    pub struct FiggyChart {
        renderer: WindowedRenderer<'static>,
        chart: Chart,
        view: ChartView,
        /// Current WebGPU surface size in physical canvas pixels. This is a
        /// viewport property, not the exported document size.
        surface_size: (u32, u32),
        series_cfgs: Vec<SeriesConfig>,
        styles: Vec<ChartStyle>,
        /// 1:1 with `series_cfgs` — legend label text (None = no legend row).
        labels: Vec<Option<String>>,
        /// True while the legend follows the wrapper's auto row layout.
        /// Direct `set_config` legend edits turn this off so later series
        /// changes preserve user-authored text instead of deleting rows.
        legend_auto_managed: bool,
        /// Registered columns: id → (len, content hash) for upsert skipping.
        columns: HashMap<String, (usize, u64)>,
        /// CPU mirror of every registered column (the same f32 values the
        /// pool holds). The renderer keeps no CPU copies of pool data, but
        /// errorbar-aware fitting needs value±err PAIRS, which per-column
        /// scalars cannot reconstruct — the upload path's staging Vec lands
        /// here instead of being dropped (no extra copy; bounded by the
        /// same data volume as the pool).
        col_data: HashMap<String, Vec<f32>>,
        /// Per-series errorbar fit extents, computed once per (series,
        /// column contents) and revalidated against `columns` signatures at
        /// fit time — column re-upserts and series redefinitions need no
        /// invalidation hooks.
        err_fit: HashMap<String, ErrFitEntry>,
        /// A removal happened — defragment once on the next frame.
        needs_defrag: bool,
        /// Monotonic color assignment for newly registered series.
        color_seq: usize,
        hitmap: HitMap,
        selected: Option<HitId>,
        dragging: bool,
        resizing: Option<ModelResizeHandle>,
        cycle: renderer::ColorCycle,
        clear_color: Color,
        view_dirty: bool,
    }

    impl FiggyChart {
        fn display_scale_and_panel(&self) -> (f32, Rect) {
            let logical = self.chart.config().chart_area.0;
            super::fit_display_panel((logical.width, logical.height), self.surface_size)
        }

        fn display_config(&self) -> (renderer::config::Config, Rect, f32) {
            super::display_config_for_surface(self.chart.config(), self.surface_size)
        }

        fn display_delta_to_document(&self, dx: f32, dy: f32) -> (f32, f32) {
            let (scale, _) = self.display_scale_and_panel();
            if scale > 0.0 {
                (dx / scale, dy / scale)
            } else {
                (dx, dy)
            }
        }

        fn label_text(&self, label: &str) -> RichText {
            let content = &self.chart.config().legend.content;
            RichText {
                segments: rich_segments_from_text(label),
                color: content.color,
                font_size: content.font_size,
                font: content.font.clone(),
            }
        }

        fn rich_text_to_plain(rt: &RichText) -> String {
            rt.segments.iter().map(|s| s.text).collect()
        }

        fn fallback_label(&self, i: usize) -> Option<RichText> {
            self.series_cfgs
                .get(i)
                .and_then(|cfg| cfg.label.clone())
                .or_else(|| self.labels.get(i)?.as_ref().map(|s| self.label_text(s)))
        }

        fn append_legend_entry_for(&mut self, i: usize) {
            let Some(cfg) = self.series_cfgs.get(i).cloned() else {
                return;
            };
            let Some(label) = self.fallback_label(i) else {
                return;
            };
            self.chart.with_decoration_change(|c| {
                renderer::config::append_legend_entry_rich(
                    &mut c.legend.content,
                    renderer::config::series_symbol_segments(&cfg),
                    label.segments,
                );
                c.legend.visible = true;
            });
        }

        fn set_legend_entry_for(&mut self, i: usize, label: RichText) {
            let Some(cfg) = self.series_cfgs.get(i).cloned() else {
                return;
            };
            self.chart.with_decoration_change(|c| {
                renderer::config::set_legend_entry_label(
                    &mut c.legend.content,
                    i,
                    renderer::config::series_symbol_segments(&cfg),
                    label.segments,
                );
                c.legend.visible = true;
            });
        }

        fn remove_legend_entry_for(&mut self, i: usize) {
            self.chart.with_decoration_change(|c| {
                if renderer::config::remove_legend_entry(&mut c.legend.content, i)
                    && c.legend.content.segments.is_empty()
                {
                    c.legend.visible = false;
                }
            });
        }

        fn sync_legend_symbols(&mut self, append_missing: bool) {
            let existing =
                renderer::config::legend_entry_count(&self.chart.config().legend.content);
            self.chart.with_decoration_change(|c| {
                renderer::config::update_legend_symbols_preserving_text(
                    &mut c.legend.content,
                    &self.series_cfgs,
                );
            });
            if append_missing {
                if existing > self.series_cfgs.len() {
                    for i in (self.series_cfgs.len()..existing).rev() {
                        self.remove_legend_entry_for(i);
                    }
                }
                for i in existing..self.series_cfgs.len() {
                    self.append_legend_entry_for(i);
                }
            }
        }

        /// Explicit reset: rebuild every auto legend row from SeriesConfig.label
        /// first, falling back to the wrapper's legacy string labels.
        fn rebuild_legend_from_series_labels(&mut self) {
            let entries: Vec<(SeriesConfig, RichText)> = self
                .series_cfgs
                .iter()
                .cloned()
                .enumerate()
                .filter_map(|(i, cfg)| Some((cfg, self.fallback_label(i)?)))
                .collect();

            self.chart.with_decoration_change(|c| {
                c.legend.content.segments.clear();
                for (cfg, label) in entries {
                    renderer::config::append_legend_entry_rich(
                        &mut c.legend.content,
                        renderer::config::series_symbol_segments(&cfg),
                        label.segments,
                    );
                }
                c.legend.visible = !c.legend.content.segments.is_empty();
            });
            self.legend_auto_managed = true;
        }

        fn rebuild_styles(&mut self) {
            let (scale, _) = self.display_scale_and_panel();
            self.styles = self
                .series_cfgs
                .iter()
                .map(|cfg| self.renderer.create_style_for_series_scaled(cfg, scale))
                .collect();
        }

        /// Errorbar contribution of one series to the (x, y) fit extents —
        /// the pairwise value±err pass that per-column min/max scalars
        /// cannot express. Walks the CPU column mirror once per (series,
        /// data) combination and caches the result; the cache is validated
        /// by column content signature, so re-upserted data or a redefined
        /// series recomputes automatically. Returns `(None, None)` for
        /// series without errorbars; a column missing from the mirror skips
        /// the extension (slot min/max only — the pre-errorbar behavior,
        /// never a wrong range). Associated fn (split borrows): called from
        /// `auto_fit_all` while other fields are also borrowed.
        fn series_err_extents(
            cfg: &SeriesConfig,
            columns: &HashMap<String, (usize, u64)>,
            col_data: &HashMap<String, Vec<f32>>,
            err_fit: &mut HashMap<String, ErrFitEntry>,
        ) -> (Option<FitExtent>, Option<FitExtent>) {
            let (err_x, err_y) = err_refs(&cfg.render_type);
            if err_x.is_none() && err_y.is_none() {
                return (None, None);
            }

            let sigs: Vec<(String, (usize, u64))> = referenced_columns(cfg)
                .into_iter()
                .map(|id| (id.to_string(), columns.get(id).copied().unwrap_or((0, 0))))
                .collect();
            if let Some(e) = err_fit.get(&cfg.series_id)
                && e.sigs == sigs
            {
                return (e.x, e.y);
            }

            let pair = |vals_id: &str, r: &ErrorRef| -> Option<FitExtent> {
                let (lo_id, hi_id) = err_cols(r);
                errorbar_extent(
                    col_data.get(vals_id)?,
                    col_data.get(lo_id)?,
                    col_data.get(hi_id)?,
                )
            };
            let x = err_x.and_then(|r| pair(&cfg.x_column, r));
            let y = err_y.and_then(|r| pair(&cfg.y_column, r));
            err_fit.insert(cfg.series_id.clone(), ErrFitEntry { sigs, x, y });
            (x, y)
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
        /// Bind the low-level chart kernel to `canvas` (uses the canvas's
        /// current pixel size). Ordinary web hosts should prefer
        /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
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
            config.chart_area = ChartArea(Rect {
                x: 0,
                y: 0,
                width: w,
                height: h,
            });
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

            Ok(FiggyChart {
                renderer,
                chart,
                view,
                surface_size: (w, h),
                series_cfgs: Vec::new(),
                styles: Vec::new(),
                labels: Vec::new(),
                legend_auto_managed: true,
                columns: HashMap::new(),
                col_data: HashMap::new(),
                err_fit: HashMap::new(),
                needs_defrag: false,
                color_seq: 0,
                hitmap: HitMap::standard_chart(),
                selected: None,
                dragging: false,
                resizing: None,
                cycle: renderer::ColorCycle::Classic,
                clear_color: Color::WHITE,
                view_dirty: false,
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
                if v < min {
                    min = v;
                }
                if v > max {
                    max = v;
                }
            }
            let column = Column {
                data: data.to_vec(),
                min,
                max,
            };
            self.renderer.add_column(id, &column).map_err(js_err)?;
            // The renderer keeps no CPU copies of pool data; this Vec was
            // headed for the drop. Keep it as the fit mirror instead —
            // errorbar-aware auto-fit re-reads value/err pairs from it.
            self.col_data.insert(id.to_string(), column.data);
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
            self.col_data.remove(id);
            self.renderer.remove_column(id);
            self.needs_defrag = true;

            let keep: Vec<bool> = self
                .series_cfgs
                .iter()
                .map(|cfg| !referenced_columns(cfg).contains(&id))
                .collect();
            if keep.iter().any(|k| !k) {
                let removed: Vec<usize> = keep
                    .iter()
                    .enumerate()
                    .filter_map(|(i, keep)| (!keep).then_some(i))
                    .collect();
                let mut it = keep.iter();
                self.series_cfgs.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.styles.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.labels.retain(|_| *it.next().unwrap());
                if self.legend_auto_managed {
                    for i in removed.into_iter().rev() {
                        self.remove_legend_entry_for(i);
                    }
                } else {
                    self.sync_legend_symbols(false);
                }
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
            let existing = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id);
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
            let label_changed = !label.is_empty();
            let rich_label = if label_changed {
                Some(self.label_text(label))
            } else {
                existing.and_then(|i| self.series_cfgs[i].label.clone())
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
                    },
                },
            };
            self.ensure_columns_exist(&cfg)?;
            let (scale, _) = self.display_scale_and_panel();
            let style = self.renderer.create_style_for_series_scaled(&cfg, scale);

            match existing {
                Some(i) => {
                    self.series_cfgs[i] = cfg;
                    self.styles[i] = style;
                    self.labels[i] = plain_label;
                    if label_changed && let Some(label) = rich_label {
                        self.set_legend_entry_for(i, label);
                    } else {
                        self.sync_legend_symbols(false);
                    }
                }
                None => {
                    self.series_cfgs.push(cfg);
                    self.styles.push(style);
                    self.labels.push(plain_label);
                    if label_changed && rich_label.is_some() {
                        self.append_legend_entry_for(self.series_cfgs.len() - 1);
                    }
                }
            }
            Ok(())
        }

        /// Set / change / remove a series' legend label. `'\n'` breaks lines;
        /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments.
        /// Empty string removes the legend row. Returns `true` when the
        /// series exists.
        pub fn set_series_label(&mut self, series_id: &str, label: &str) -> bool {
            let Some(i) = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id)
            else {
                return false;
            };
            if label.is_empty() {
                self.labels[i] = None;
                self.series_cfgs[i].label = None;
                self.remove_legend_entry_for(i);
            } else {
                let label = self.label_text(label);
                self.labels[i] = Some(Self::rich_text_to_plain(&label));
                self.series_cfgs[i].label = Some(label.clone());
                self.set_legend_entry_for(i, label);
            }
            true
        }

        /// Unregister a series (and its legend row). Columns stay registered.
        /// Returns `true` when the series existed.
        pub fn remove_series(&mut self, series_id: &str) -> bool {
            let Some(i) = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id)
            else {
                return false;
            };
            self.series_cfgs.remove(i);
            self.styles.remove(i);
            self.labels.remove(i);
            self.err_fit.remove(series_id);
            if self.legend_auto_managed {
                self.remove_legend_entry_for(i);
            } else {
                self.sync_legend_symbols(false);
            }
            true
        }

        /// Fit the x axis to a column's range with proportional padding.
        pub fn auto_fit_x(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            self.chart
                .auto_fit_x(self.renderer.pool(), column, padding)
                .map_err(js_err)
        }

        pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            self.chart
                .auto_fit_y(self.renderer.pool(), column, padding)
                .map_err(js_err)
        }

        /// Fit BOTH axes to the union of every registered series, leaving a
        /// uniform `padding` fraction of the data span as margin on each
        /// side (`0.0` = exact fit, `0.05` = 5% top/bottom/left/right).
        /// This is the whole fit policy — no rounding of the range ends;
        /// ticks land on nice values inside the range by themselves. Hosts
        /// should call this instead of re-deriving ranges.
        ///
        /// Errorbar series contribute their full bar extents
        /// (`value − err_lo .. value + err_hi`, the exact GPU arithmetic),
        /// so caps never clip against an auto-fitted range. That pairwise
        /// pass runs once per (series, data) and is cached — repeat fits
        /// stay metadata-cheap.
        pub fn auto_fit_all(&mut self, padding: f64) -> Result<(), JsValue> {
            if self.series_cfgs.is_empty() {
                return Ok(());
            }
            let mut x_ext = FitExtent::EMPTY;
            let mut y_ext = FitExtent::EMPTY;
            for cfg in &self.series_cfgs {
                let pool = self.renderer.pool();
                x_ext.union(&Chart::slot_extent(pool, &cfg.x_column).map_err(js_err)?);
                y_ext.union(&Chart::slot_extent(pool, &cfg.y_column).map_err(js_err)?);
                let (ex, ey) =
                    Self::series_err_extents(cfg, &self.columns, &self.col_data, &mut self.err_fit);
                if let Some(e) = ex {
                    x_ext.union(&e);
                }
                if let Some(e) = ey {
                    y_ext.union(&e);
                }
            }
            self.chart.auto_fit_x_extent(&x_ext, padding);
            self.chart.auto_fit_y_extent(&y_ext, padding);
            Ok(())
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
            self.chart
                .with_decoration_change(|c| c.apply_axis_preset(p));
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
            self.sync_legend_symbols(false);
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
            let legend_content_changed =
                self.chart.config().legend.content != new_cfg.legend.content;
            *self.chart.config_mut() = new_cfg;
            if legend_content_changed {
                self.legend_auto_managed = false;
            }
            self.view_dirty = true;
            self.rebuild_styles();
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
                    cfg.label
                        .as_ref()
                        .map(Self::rich_text_to_plain)
                        .or_else(|| {
                            self.series_cfgs
                                .iter()
                                .position(|old| old.series_id == cfg.series_id)
                                .and_then(|i| self.labels[i].clone())
                        })
                })
                .collect();
            self.series_cfgs = new_series;
            self.color_seq = self.series_cfgs.len().max(self.color_seq);
            self.ensure_zero_column()?;
            self.rebuild_styles();
            self.sync_legend_symbols(self.legend_auto_managed);
            Ok(())
        }

        /// Explicitly rebuild the auto legend from `SeriesConfig.label`.
        /// Legacy string labels are used only when a series has no rich label.
        pub fn reset_legend_from_series_labels(&mut self) {
            self.rebuild_legend_from_series_labels();
        }

        // ---- pointer interaction (coordinates in canvas pixels) ----

        /// Hit-test the chart chrome at canvas pixel `(x, y)` — returns the
        /// topmost element's stable id (`"data_area"`, `"axis_bottom"`,
        /// `"tick_labels_left"`, `"axis_title_left"`, `"legend"`,
        /// `"chart_title"`, …) or `null`. Pure geometry, no selection state
        /// change: the renderer's own layout answers, so hosts don't have to
        /// re-derive box positions for hover cursors / context UI.
        pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
            let (display_config, _, _) = self.display_config();
            self.hitmap
                .hit_test(
                    &display_config,
                    &CpuTextMeasure::for_style(&display_config.draw_style),
                    x,
                    y,
                )
                .and_then(|id| self.hitmap.get(id))
                .map(|el| el.element_id())
        }

        /// Pick the nearest visible data primitive to canvas pixel `(x, y)`.
        /// Scatter hits use the visible marker size, including per-point style
        /// mapping; line strokes snap to the nearest endpoint data point on
        /// the hit segment. Errorbar stems/caps are not pick targets.
        /// Returns JSON `{ source_id, series_id, point_index, data_x, data_y,
        /// distance_px }`, or `null` when no visible primitive is within
        /// `max_distance_px`.
        pub fn pick_point(
            &self,
            x: f32,
            y: f32,
            max_distance_px: f32,
        ) -> Result<Option<String>, JsValue> {
            let (display_config, _, _) = self.display_config();
            let columns = WebPointColumns {
                col_data: &self.col_data,
            };
            let Some(picked) = renderer::pick_nearest_point(
                &display_config,
                &self.series_cfgs,
                &columns,
                x,
                y,
                PointPickOptions { max_distance_px },
            ) else {
                return Ok(None);
            };

            crate::picked_point_json_string(&picked)
                .map(Some)
                .map_err(js_err)
        }

        /// Replace the picked-point overlay config. Passing JSON `null`
        /// clears it.
        pub fn set_picked_points(&mut self, json: &str) -> Result<(), JsValue> {
            let picked: Option<renderer::config::PickedPointsConfig> =
                serde_json::from_str(json).map_err(js_err)?;
            self.chart.with_decoration_change(|c| {
                c.picked_points = picked;
            });
            Ok(())
        }

        /// Pointer press. Returns `true` while something is selected.
        /// The host can mirror that state in its own UI.
        pub fn on_press(&mut self, x: f32, y: f32) -> bool {
            let (display_config, _, _) = self.display_config();
            // Resize handles on the selected element win over hit-testing.
            if let Some(id) = self.selected
                && let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                && let Some(handle) = rz.hit_resize_handle(
                    &display_config,
                    &CpuTextMeasure::for_style(&display_config.draw_style),
                    x,
                    y,
                )
            {
                self.resizing = Some(handle);
                self.dragging = false;
                return true;
            }
            self.resizing = None;

            let new_sel = self.hitmap.hit_test(
                &display_config,
                &CpuTextMeasure::for_style(&display_config.draw_style),
                x,
                y,
            );
            self.dragging = new_sel.is_some_and(|id| {
                self.hitmap
                    .get(id)
                    .is_some_and(|el| el.as_draggable().is_some())
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
            let (dx, dy) = self.display_delta_to_document(dx, dy);
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

            // Browser resize is preview zoom, not a document mutation. The
            // stored chart remains the export SSoT; the live canvas renders a
            // scaled/letterboxed display chart derived from it.
            let (display_config, panel_rect, _) = self.display_config();
            let display_chart = Chart::new(display_config);
            let view_dirty = self.view_dirty;
            let raster_dirty = self.chart.consume_raster_dirty();
            // `Renderer::prepare` (inside `draw` below) rewrites the transform
            // uniform from the current config every frame; the data-dirty flag
            // only needs resetting.
            let _ = self.chart.consume_data_dirty();
            if view_dirty || raster_dirty {
                let sel_boxes: Vec<SelectionBox> = self
                    .selected
                    .and_then(|id| {
                        self.hitmap.selection_box(
                            id,
                            display_chart.config(),
                            &CpuTextMeasure::for_style(&display_chart.config().draw_style),
                        )
                    })
                    .into_iter()
                    .collect();
                self.renderer
                    .refresh_axis_with_selection(
                        &mut self.view,
                        &display_chart,
                        panel_rect,
                        &sel_boxes,
                    )
                    .map_err(js_err)?;
                self.view_dirty = false;
            }

            let series: Vec<Series<'_>> = self
                .series_cfgs
                .iter()
                .zip(self.styles.iter())
                .map(|(cfg, style)| Series { config: cfg, style })
                .collect();
            let items = [ChartDrawItem {
                view: &self.view,
                chart_config: display_chart.config(),
                series: &series,
            }];
            self.renderer.draw(self.clear_color, &items).map_err(js_err)
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
        }

        /// Resize the swap chain viewport. The chart Config keeps its
        /// document/export `chart_area`; live rendering scales that logical
        /// document into this surface.
        pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
            let (w, h) = (width.max(1), height.max(1));
            self.renderer.resize(w, h).map_err(js_err)?;
            self.surface_size = (w, h);
            self.view_dirty = true;
            self.rebuild_styles();
            Ok(())
        }

        /// Export the panel as PNG bytes at `scale ×` resolution.
        /// JS: `const png = await chart.export_png(2.0);`
        /// (`&mut self`: the renderer's export runs its prepare phase —
        /// transform uniforms + arc-prefix compute; wasm-bindgen serializes
        /// access, so this changes nothing for JS callers.)
        pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
            let bytes = self
                .renderer
                .export_panel_png_bytes_with_clear_async(
                    &self.chart,
                    &self.series_cfgs,
                    scale,
                    self.clear_color,
                )
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
