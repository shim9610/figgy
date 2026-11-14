//! Axes, titles, legend, colorbar, annotations, decoration bounds and
//! auto-fit.

use super::*;
use renderer::theme::SubplotScope;

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
    Hidden,
    Grotrian,
}

impl From<AxisPreset> for renderer::AxisPreset {
    fn from(p: AxisPreset) -> Self {
        match p {
            AxisPreset::BoxedInward => Self::BoxedInward,
            AxisPreset::BoxedOutward => Self::BoxedOutward,
            AxisPreset::OpenOutward => Self::OpenOutward,
            AxisPreset::OpenInward => Self::OpenInward,
            AxisPreset::Minimal => Self::Minimal,
            AxisPreset::Hidden => Self::Hidden,
            AxisPreset::Grotrian => Self::Grotrian,
        }
    }
}

/// The variant name of an annotation kind, as its JSON key.
pub(crate) fn annotation_kind_name(kind: &renderer::AnnotationKind) -> &'static str {
    match kind {
        renderer::AnnotationKind::ReferenceLine { .. } => "ReferenceLine",
        renderer::AnnotationKind::Span { .. } => "Span",
        renderer::AnnotationKind::Text { .. } => "Text",
        renderer::AnnotationKind::Arrow { .. } => "Arrow",
        renderer::AnnotationKind::Rect { .. } => "Rect",
        renderer::AnnotationKind::Ellipse { .. } => "Ellipse",
        renderer::AnnotationKind::Polygon { .. } => "Polygon",
    }
}

/// The vertex index a polygon handle name (`"v3"`) names.
pub(crate) fn vertex_handle_index(name: &str) -> Option<usize> {
    name.strip_prefix('v')?.parse().ok()
}

/// `RenderWarning::AnnotationSkipped` entries as `[{index, reason}]`.
pub(crate) fn annotation_warnings_json(
    warnings: &[renderer::RenderWarning],
) -> Result<String, JsValue> {
    let values = warnings
        .iter()
        .filter_map(|w| match w {
            renderer::RenderWarning::AnnotationSkipped { index, reason } => {
                Some(serde_json::json!({ "index": index, "reason": format!("{reason:?}") }))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    serde_json::to_string(&values).map_err(js_err)
}

#[wasm_bindgen]
impl FiggyChart {
    /// Fit the x axis to a column's range with proportional padding.
    pub fn auto_fit_x(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
        let mut chart = Chart::new(self.chart_config()?.clone());
        chart
            .auto_fit_x(self.renderer.pool(), column, padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
        let mut chart = Chart::new(self.chart_config()?.clone());
        chart
            .auto_fit_y(self.renderer.pool(), column, padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    /// Fit overlay layer `layer`'s Y axis to a column's cached statistics
    /// with proportional padding, the overlay counterpart of `auto_fit_y`.
    /// With the overlay's `link_y` on it fits the chart's Y axes it
    /// follows. A layer index past the overlay list is an error.
    /// JS: `chart.auto_fit_overlay_y(0, "pressure", 0.05)`
    pub fn auto_fit_overlay_y(
        &mut self,
        layer: usize,
        column: &str,
        padding: f64,
    ) -> Result<(), JsValue> {
        if layer >= self.chart_config()?.overlays.len() {
            return Err(js_err("overlay layer index is out of range"));
        }
        let mut chart = Chart::new(self.chart_config()?.clone());
        chart
            .auto_fit_overlay_y_union(self.renderer.pool(), layer, &[column], padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    /// Fit overlay layer `layer`'s own X axis (`OverlayLayer::x_axis`) to a
    /// column's cached statistics with proportional padding, the X
    /// counterpart of `auto_fit_overlay_y`. With the overlay's `link_x` on
    /// it fits the chart's X axes it follows. A layer index past the
    /// overlay list is an error.
    /// JS: `chart.auto_fit_overlay_x(0, "time_b", 0.05)`
    pub fn auto_fit_overlay_x(
        &mut self,
        layer: usize,
        column: &str,
        padding: f64,
    ) -> Result<(), JsValue> {
        if layer >= self.chart_config()?.overlays.len() {
            return Err(js_err("overlay layer index is out of range"));
        }
        let mut chart = Chart::new(self.chart_config()?.clone());
        chart
            .auto_fit_overlay_x_union(self.renderer.pool(), layer, &[column], padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    /// Fit subplot `index`'s X (`axis = "x"`) or Y (`axis = "y"`) axis to
    /// the union of the given columns' cached statistics with
    /// proportional padding. A linked axis moves its whole link group.
    /// An index past the subplot list or another axis name is an error.
    /// JS: `chart.auto_fit_subplot(0, "y", ["flow"], 0.05)`
    pub fn auto_fit_subplot(
        &mut self,
        index: usize,
        axis: &str,
        columns: Vec<String>,
        padding: f64,
    ) -> Result<(), JsValue> {
        if index >= self.chart_config()?.subplots.len() {
            return Err(js_err("subplot index is out of range"));
        }
        let ids: Vec<&str> = columns.iter().map(String::as_str).collect();
        let mut chart = Chart::new(self.chart_config()?.clone());
        match axis {
            "x" => chart.auto_fit_subplot_x_union(self.renderer.pool(), index, &ids, padding),
            "y" => chart.auto_fit_subplot_y_union(self.renderer.pool(), index, &ids, padding),
            _ => return Err(js_err("axis must be \"x\" or \"y\"")),
        }
        .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    /// Fit the axes of the active layer to the union of that layer's
    /// series, leaving a uniform `padding` fraction of the data span as
    /// margin on each side (`0.0` = no padding, `0.05` = 5%
    /// top/bottom/left/right). This is the whole fit policy — no rounding
    /// of the range ends; ticks land on nice values inside the range by
    /// themselves. Hosts should call this instead of re-deriving ranges.
    ///
    /// The active layer (`set_active_layer`) decides which axes move:
    /// `Base` and `Annotations` fit the parent's `bottom_x` and `left_y`,
    /// `Subplot(i)` fits subplot `i`'s, and `Overlay(n)` /
    /// `SubplotOverlay(i, n)` fit that overlay's own axes: its Y axis, and
    /// its X axis while its `link_x` is off. An axis fits every series
    /// drawn against it (`Config::layer_of_series`). A linked axis
    /// (`link_x`, `link_y` of a subplot or an overlay) is one axis shared
    /// by its link group: it fits the series of every chart and overlay in
    /// the group and the range moves the whole group, whichever member is
    /// active. X and Y links are followed separately. By default an overlay
    /// follows its chart's X axis and has its own Y axis, so its series
    /// count towards that X axis and not towards `left_y`. An axis that is not linked fits its own chart's
    /// series only; to fit every layer, set each layer active in turn and
    /// call this again.
    ///
    /// Each series contributes its exact original GPU primitive domain:
    /// valid adjacent segments for line-only mode, paired finite points
    /// for point-bearing modes, and enabled error endpoints. The reduction
    /// is compiled on first fit (`createComputePipelineAsync` on wasm),
    /// and the renderer reuses a series' extent while its columns are
    /// unchanged (`begin_auto_fit_all`). Matrix-backed series use the same
    /// engine to return the
    /// exact cell/sample-lattice bounds; only histograms remain a metadata
    /// union because their uploaded edge column already is their geometry.
    /// Series registration itself does not submit GPU work.
    pub async fn auto_fit_all(&mut self, padding: f64) -> Result<(), JsValue> {
        // A monotonic chart is auto-scaled by its full call (#271).
        if self.monotonic.is_some() {
            return Err(js_err(
                "a monotonic chart auto-scales with set_monotonic(..., \"autoscale\")",
            ));
        }
        if self.request_stream_auto_fit(padding)? {
            self.request_host_redraw();
            return Ok(());
        }
        let ticket = match self.renderer.begin_auto_fit_all(self.chart_id, padding) {
            Err(renderer::GpuExtentError::EngineNotReady) => {
                // The first fit that needs a reduction compiles the engine.
                self.renderer
                    .ensure_errorbar_extent_engine()
                    .await
                    .map_err(js_err)?;
                self.renderer
                    .begin_auto_fit_all(self.chart_id, padding)
                    .map_err(js_err)?
            }
            ticket => ticket.map_err(js_err)?,
        };
        let resolved = ticket.resolve().await.map_err(js_err)?;
        self.renderer
            .commit_auto_fit_all(resolved)
            .map_err(js_err)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Fit the shared z axis to every column the colourbar maps: matrix
    /// value columns, scatter `point_color_map` columns, vector colour
    /// maps and streamline colour columns. A chart without a colourbar is left unchanged.
    pub fn auto_fit_colorbar(&mut self, padding: f64) -> Result<(), JsValue> {
        let mut z_extent = FitExtent::EMPTY;
        for cfg in self.chart_series()? {
            let mut union = |id: &str| -> Result<(), JsValue> {
                z_extent.union(&Chart::slot_extent(self.renderer.pool(), id).map_err(js_err)?);
                Ok(())
            };
            match &cfg.render_type {
                DataRenderType::Field(renderer::data_config::FieldSeries { matrix, .. }) => {
                    for id in &matrix.columns {
                        union(id)?;
                    }
                }
                // A mesh colours by its value column (#210).
                DataRenderType::TriMesh(mesh) => union(&mesh.values.column)?,
                DataRenderType::Xy(renderer::data_config::XySeries {
                    scatter: Some(scatter),
                    ..
                }) => {
                    if let Some(map) = &scatter.point_color_map {
                        union(&map.column)?;
                    }
                }
                DataRenderType::Ternary {
                    scatter: Some(scatter),
                    ..
                } => {
                    if let Some(map) = &scatter.point_color_map {
                        union(&map.column)?;
                    }
                }
                // A vector's colour map: a column or a grid's columns.
                DataRenderType::Vector { .. } => match vector_columns(&cfg.render_type) {
                    Some(VectorColumns::Points { color, .. }) => {
                        if let Some(color) = color {
                            union(color)?;
                        }
                    }
                    Some(VectorColumns::Grid { color, .. }) => {
                        if let Some(grid) = color {
                            for id in &grid.columns {
                                union(id)?;
                            }
                        }
                    }
                    None => {}
                },
                DataRenderType::Streamlines { color_map, .. } => {
                    if let Some(map) = color_map {
                        union(&map.column)?;
                    }
                }
                // A Feynman series maps no value (#234), nor does a
                // monotonic one (#271).
                DataRenderType::Feynman(_) | DataRenderType::MonotonicLine(_) => {}
                // An energy-level series' mapped transition values are SSOT
                // (#203): each finite one widens the extent.
                DataRenderType::EnergyLevels(levels) => {
                    if levels.style.transition.color_by_value {
                        for value in levels.transitions.iter().filter_map(|t| t.value) {
                            if value.is_finite() {
                                z_extent.union(&FitExtent {
                                    min: value,
                                    max: value,
                                    min_positive: (value > 0.0).then_some(value),
                                });
                            }
                        }
                    }
                }
                // A parallel series' colour column maps through the colourbar.
                DataRenderType::Parallel { color_column, .. } => {
                    if let Some(column) = color_column {
                        union(column)?;
                    }
                }
                // An image's colours are its data: the colourbar maps none
                // of its channels.
                DataRenderType::RgbImage { .. }
                | DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, .. })
                | DataRenderType::Histogram { .. }
                | DataRenderType::CategoryBars { .. }
                | DataRenderType::BoxPlot { .. }
                | DataRenderType::RangeBars { .. }
                | DataRenderType::Radar { .. }
                | DataRenderType::Ternary { scatter: None, .. } => {}
            }
        }
        let mut chart = Chart::new(self.chart_config()?.clone());
        chart.auto_fit_colorbar_extent(&z_extent, padding);
        self.replace_chart_config(chart.config().clone())?;
        self.request_host_redraw();
        Ok(())
    }

    // ---- titles ----

    /// Set the title text and grow its band if needed. Like every text
    /// setter (`set_x_title`, `set_y_title`, `set_colorbar_title`, and the
    /// labels of `add_line_series` and `set_series_label`), it reads the
    /// text as matplotlib reads a label: `$…$`, `\(…\)` and `$$…$$` spans
    /// are TeX, `\$` is a dollar sign, and text whose TeX does not parse is
    /// shown as written. For a text-only edit that keeps the exact margin,
    /// use `get_config` / `set_config` and change only `chart_title.text`.
    pub fn set_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        set_chart_title_label(&mut config, text);
        self.replace_chart_config(config)
    }

    /// Set the bottom x axis title, a label with `$…$` spans as TeX.
    pub fn set_x_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        config.bottom_x.title_option.text.segments = rich_segments_from_label(text);
        self.replace_chart_config(config)
    }

    /// Set the left y axis title, a label with `$…$` spans as TeX.
    pub fn set_y_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        config.left_y.title_option.text.segments = rich_segments_from_label(text);
        self.replace_chart_config(config)
    }

    /// Set the colourbar title text, a label with `$…$` spans as TeX, and
    /// make it visible. An empty string
    /// hides it. The full rich-text/title offsets remain available through
    /// `set_colorbar_axis` or the chart Config SSoT.
    pub fn set_colorbar_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        let bar = config
            .colorbar
            .as_mut()
            .ok_or_else(|| js_err("colorbar title needs Config.colorbar"))?;
        bar.axis.title_option.text.segments = rich_segments_from_label(text);
        bar.axis.title_option.visible = !text.is_empty();
        self.replace_chart_config(config)
    }

    /// Replace the colourbar's z-axis options as one SSoT value. This is
    /// the focused edit boundary for tick direction/style/length, value
    /// direction, tick-label style/offsets, and title options.
    pub fn set_colorbar_axis(&mut self, json: &str) -> Result<(), JsValue> {
        let axis: renderer::config::AxisOptions = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        let bar = config
            .colorbar
            .as_mut()
            .ok_or_else(|| js_err("colorbar axis needs Config.colorbar"))?;
        bar.axis = axis;
        self.replace_chart_config(config)
    }

    // ---- presets ----

    /// Apply an axis frame preset to all four axes and every overlay's own
    /// axes (decoration-only).
    pub fn apply_axis_preset(&mut self, preset: AxisPreset) -> Result<(), JsValue> {
        let p: renderer::AxisPreset = preset.into();
        let mut config = self.chart_config()?.clone();
        config.apply_axis_preset(p);
        self.replace_chart_config(config)
    }

    /// Switch the series color rotation: recolors every series in order,
    /// rebuilds their GPU styles, and keeps legend swatches in sync.
    pub fn apply_color_cycle(&mut self, cycle: ColorCycle) -> Result<(), JsValue> {
        let cycle = renderer::ColorCycle::from(cycle);
        let mut series = self.chart_series()?.to_vec();
        for (i, cfg) in series.iter_mut().enumerate() {
            cycle.apply_to_series(cfg, i);
        }
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        renderer::config::update_legend_symbols_preserving_text(
            &mut config.legend.content,
            &series,
            config.colorbar.as_ref().map(|bar| &bar.colormap),
        );
        let count = series.len();
        self.replace_chart_state(config, series)?;
        self.cycle = cycle;
        self.color_seq = count;
        Ok(())
    }

    /// Explicitly rebuild the auto legend from `SeriesConfig.label`.
    /// Legacy string labels are used only when a series has no rich label.
    pub fn reset_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
        self.rebuild_legend_from_series_labels()
    }

    pub(super) fn set_selected_annotation(&mut self, selected: Option<usize>) {
        if self.selected_annotation != selected {
            self.selected_annotation = selected;
            self.annotation_selection_dirty = true;
            self.redraw_pending = true;
        }
    }

    pub(super) fn annotation_hit_id(&self, index: usize) -> Result<HitId, JsValue> {
        self.view
            .annotation_map()
            .find(index)
            .ok_or_else(|| js_err("annotation is not drawn in the current frame"))
    }

    /// Move an annotation or one of its handles by a delta in physical
    /// canvas pixels, the frame's anchor space. `Ok(false)` for a
    /// rejected move; the config is unchanged then.
    pub(super) fn move_annotation(
        &mut self,
        index: usize,
        dx: f32,
        dy: f32,
        handle: Option<ModelResizeHandle>,
    ) -> Result<bool, JsValue> {
        let id = self.annotation_hit_id(index)?;
        let map = self.view.annotation_map();
        let mut config = self.chart_config()?.clone();
        let result = match handle {
            Some(h) => map.resize_by(&mut config.annotations, id, h, dx, dy),
            None => map.drag_by(&mut config.annotations, id, dx, dy),
        };
        if result != Some(NudgeResult::Moved) {
            return Ok(false);
        }
        self.replace_chart_config(config)?;
        Ok(true)
    }

    // ---- annotation layer ----

    /// The drawn annotations of the current frame as JSON: `index`, `id`,
    /// `kind`, the selection box `rect` and its `handles` in physical
    /// canvas pixels, and `selected`. Items the raster skipped are
    /// absent here and listed by `annotation_warnings`.
    pub fn annotations(&self) -> Result<String, JsValue> {
        let map = self.view.annotation_map();
        let items = &self.chart_config()?.annotations.items;
        let values = map
                .entries()
                .filter_map(|(id, e)| {
                    let item = items.get(e.index)?;
                    let b = map.selection_box(id)?;
                    let r = b.rect();
                    Some(serde_json::json!({
                        "index": e.index,
                        "id": item.id,
                        "element": e.id().element_id(),
                        "kind": annotation_kind_name(&item.kind),
                        "rect": [r.x, r.y, r.width, r.height],
                        "handles": b.handles().iter().map(|r| vec![r.x, r.y, r.width, r.height]).collect::<Vec<_>>(),
                        "selected": self.selected_annotation == Some(e.index),
                    }))
                })
                .collect::<Vec<_>>();
        serde_json::to_string(&values).map_err(js_err)
    }

    /// The topmost drawn annotation under a point in physical canvas
    /// pixels, as its index.
    pub fn hit_annotation(&self, x: f32, y: f32) -> Option<u32> {
        let map = self.view.annotation_map();
        let id = map.hit_test([x, y])?;
        map.element(id).map(|e| e.index as u32)
    }

    /// Select one annotation (its box and handles are drawn on the
    /// overlay) or none. Selecting an annotation clears the decoration
    /// selection; selecting a decoration with `on_press` clears this.
    pub fn select_annotation(&mut self, index: Option<u32>) -> Result<(), JsValue> {
        let index = index.map(|i| i as usize);
        if let Some(i) = index
            && i >= self.chart_config()?.annotations.items.len()
        {
            return Err(js_err("unknown annotation"));
        }
        if index.is_some() && self.chart_selection()?.is_some() {
            self.set_chart_selection(None)?;
        }
        self.set_selected_annotation(index);
        Ok(())
    }

    pub fn selected_annotation(&self) -> Option<u32> {
        self.selected_annotation.map(|i| i as u32)
    }

    /// The compass name (`"W"`, `"E"`, ...) of the annotation's handle
    /// under a point, or none. Only the handles `annotations` lists are
    /// live: a span's two edges, a line's two value handles, an arrow's
    /// two endpoints.
    pub fn annotation_handle(&self, index: u32, x: f32, y: f32) -> Option<String> {
        // An annotation not drawn in the current frame has no handle.
        let id = self.view.annotation_map().find(index as usize)?;
        self.view
            .annotation_map()
            .hit_resize_handle(id, [x, y])
            .map(|h| format!("{h:?}"))
    }

    /// The vertex handle (`"v<i>"`) of a polygon annotation under a
    /// point, or none: a polygon's handles are its vertices.
    pub fn annotation_vertex(&self, index: u32, x: f32, y: f32) -> Option<String> {
        // An annotation not drawn in the current frame has no handle.
        let id = self.view.annotation_map().find(index as usize)?;
        self.view
            .annotation_map()
            .hit_vertex(id, [x, y])
            .map(|i| format!("v{i}"))
    }

    /// Move an annotation, or one of its handles, by a pointer delta in
    /// physical canvas pixels. The delta is converted back into the
    /// item's anchors through the frame's anchor space and written to
    /// the config; a move that would leave the plot or panel is rejected
    /// and returns `false`. A handle is a compass name (`"W"`, `"NE"`, ...)
    /// or a polygon vertex (`"v3"`).
    pub fn drag_annotation(
        &mut self,
        index: u32,
        dx: f32,
        dy: f32,
        handle: Option<String>,
    ) -> Result<bool, JsValue> {
        if !dx.is_finite() || !dy.is_finite() {
            return Err(js_err("drag must be finite"));
        }
        if let Some(vertex) = handle.as_deref().and_then(vertex_handle_index) {
            let id = self.annotation_hit_id(index as usize)?;
            let map = self.view.annotation_map();
            let mut config = self.chart_config()?.clone();
            let result = map.drag_vertex(&mut config.annotations, id, vertex, dx, dy);
            if result != Some(NudgeResult::Moved) {
                return Ok(false);
            }
            self.replace_chart_config(config)?;
            return Ok(true);
        }
        use ModelResizeHandle::*;
        let handle = handle
            .map(|name| {
                [N, S, W, E, NW, NE, SW, SE]
                    .into_iter()
                    .find(|h| format!("{h:?}") == name)
                    .ok_or_else(|| js_err("unknown resize handle"))
            })
            .transpose()?;
        self.move_annotation(index as usize, dx, dy, handle)
    }

    /// Append an `Annotation` (JSON, the same shape as in `get_config`)
    /// and return its index.
    pub fn add_annotation(&mut self, json: &str) -> Result<u32, JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        config.annotations.items.push(item);
        let index = config.annotations.items.len() - 1;
        self.replace_chart_config(config)?;
        Ok(index as u32)
    }

    /// Append a text note at the plot's `corner` (a `NoteAlign` name:
    /// `"TopLeft"`, `"Top"`, ..., `"BottomRight"`), its block inside the
    /// plot, and return its index (#258). `text` is a `RichText` JSON value,
    /// the same shape as in `get_config`; the note owns it, such as the
    /// unit of the chart's values. It is an ordinary text annotation: move,
    /// hide or edit it as one.
    /// JS: `const i = chart.add_corner_note(textJson, "BottomRight");`
    pub fn add_corner_note(&mut self, text: &str, corner: &str) -> Result<u32, JsValue> {
        let text: renderer::text::RichText = serde_json::from_str(text).map_err(js_err)?;
        let corner: renderer::NoteAlign =
            serde_json::from_value(serde_json::Value::String(corner.into()))
                .map_err(|_| js_err("unknown note corner"))?;
        let mut config = self.chart_config()?.clone();
        let index = config.annotations.add_corner_note(text, corner);
        self.replace_chart_config(config)?;
        Ok(index as u32)
    }

    /// Replace the annotation at `index` with an `Annotation` JSON value.
    pub fn set_annotation(&mut self, index: u32, json: &str) -> Result<(), JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        let slot = config
            .annotations
            .items
            .get_mut(index as usize)
            .ok_or_else(|| js_err("unknown annotation"))?;
        *slot = item;
        self.replace_chart_config(config)
    }

    /// Remove the annotation at `index`. Later items move down one
    /// index; the selection follows or clears.
    pub fn remove_annotation(&mut self, index: u32) -> Result<(), JsValue> {
        let index = index as usize;
        let mut config = self.chart_config()?.clone();
        if index >= config.annotations.items.len() {
            return Err(js_err("unknown annotation"));
        }
        config.annotations.items.remove(index);
        self.replace_chart_config(config)?;
        let selected = match self.selected_annotation {
            Some(i) if i == index => None,
            Some(i) if i > index => Some(i - 1),
            other => other,
        };
        self.set_selected_annotation(selected);
        Ok(())
    }

    /// Show or hide the whole annotation layer. Hiding skips the two
    /// composite draws and re-rasterizes nothing.
    pub fn set_annotations_visible(&mut self, visible: bool) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        config.annotations.visible = visible;
        self.replace_chart_config(config)
    }

    /// The anchor (`Anchor` JSON) at a point in physical canvas pixels,
    /// as `"Data"` or `"Ratio"`, for placing a new text box or arrow
    /// where the user clicked. `null` when the point has no data
    /// position (outside a log axis, or the chart has no value axes).
    pub fn pointer_to_anchor(&self, x: f32, y: f32, kind: &str) -> Result<String, JsValue> {
        let kind = match kind {
            "Data" => renderer::AnchorKind::Data,
            "Ratio" => renderer::AnchorKind::Ratio,
            _ => return Err(js_err("anchor kind must be \"Data\" or \"Ratio\"")),
        };
        use renderer::AnnotationSpace;
        let anchor = self
            .view
            .annotation_map()
            .space()
            .px_to_anchor([x, y], kind);
        serde_json::to_string(&anchor).map_err(js_err)
    }

    /// Annotations the current raster skipped, as `[{index, reason}]`
    /// with `reason` one of `NonFinite`, `NoValueAxis`, `OutOfRange`,
    /// `EmptyText`.
    pub fn annotation_warnings(&self) -> Result<String, JsValue> {
        annotation_warnings_json(self.view.render_warnings())
    }

    /// Renderer-measured decoration bounds in physical canvas pixels.
    /// An `"annotation_<n>"` id that `hit_test` reports resolves to the
    /// drawn item's selection box.
    pub fn decoration_bounds(&self, target: &str) -> Result<String, JsValue> {
        let (config, _, _) = self.display_config()?;
        let annotations = self.view.annotation_map();
        if let Some(b) = annotations
            .entries()
            .find(|(_, e)| e.id().element_id() == target)
            .and_then(|(id, _)| annotations.selection_box(id))
        {
            let r = b.rect();
            let value = serde_json::json!({
                "rect": [r.x, r.y, r.width, r.height],
                "handles": b.handles().iter().map(|r| vec![r.x, r.y, r.width, r.height]).collect::<Vec<_>>(),
            });
            return serde_json::to_string(&value).map_err(js_err);
        }
        let id = self.hitmap.find(target);
        let measure = CpuTextMeasure::for_style(&config.draw_style);
        let value = id
                .and_then(|id| self.hitmap.selection_box(id, &config, &measure))
                .map(|b| {
                    let r = b.rect();
                    let mut value = serde_json::json!({
                        "rect": [r.x, r.y, r.width, r.height],
                        "handles": b.handles().iter().map(|r| vec![r.x, r.y, r.width, r.height]).collect::<Vec<_>>(),
                    });
                    // An element that outlines its parts (polar tick labels,
                    // the polar ray) gets one polygon per part: the shapes
                    // the canvas strokes and the hit test answers within.
                    if let renderer::SelectionOutline::Parts(parts) = b.outline() {
                        let outline: Vec<Vec<[f32; 2]>> = parts
                            .iter()
                            .map(|part| renderer::select::part_outline(part).corners().to_vec())
                            .collect();
                        value["outline"] = serde_json::json!(outline);
                    }
                    value
                });
        serde_json::to_string(&value).map_err(js_err)
    }
}

// ------------------------------------------------------------------
// Subplot frames (#205): where each subplot sits and its plot background.
// ------------------------------------------------------------------

#[wasm_bindgen]
impl FiggyChart {
    /// Place subplot `index`'s frame at `(x, y)` with `width` x `height`, in
    /// document pixels (the coordinates of `get_config`'s `chart_area`s):
    /// anywhere, inside the parent chart (an inset), over another panel or
    /// beside every one. The subplot's plot must fit in the frame; an error
    /// leaves the document unchanged. A pointer drag on the frame does the
    /// same on the base layer.
    /// JS: `chart.set_subplot_area(0, 120, 80, 320, 240);`
    pub fn set_subplot_area(
        &mut self,
        index: usize,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<(), JsValue> {
        let mut config = self.chart_config()?.clone();
        let area = renderer::layout::ChartArea(Rect {
            x,
            y,
            width,
            height,
        });
        config.set_subplot_area(index, area).map_err(|error| {
            js_err(format!("subplot {index} cannot take that frame: {error:?}"))
        })?;
        self.replace_chart_config(config)
    }

    /// Subplot `index`'s frame as JSON `{"x":..,"y":..,"width":..,"height":..}`
    /// in document pixels.
    pub fn subplot_area(&self, index: usize) -> Result<String, JsValue> {
        let subplots = &self.chart_config()?.subplots;
        let subplot = subplots
            .get(index)
            .ok_or_else(|| subplot_index_error(index, subplots.len()))?;
        serde_json::to_string(&subplot.config.chart_area.0).map_err(js_err)
    }

    /// Fill the chart area with a colour under everything the chart draws
    /// (JSON `{"r":1,"g":1,"b":1,"a":1}`), or leave it transparent (`null`)
    /// so the page or the export's clear colour shows through (#265).
    pub fn set_background(&mut self, json: &str) -> Result<(), JsValue> {
        let color: Option<Color> = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        config.background = color;
        self.replace_chart_config(config)
    }

    /// Apply the theme `name` (#291, #295; the names are in
    /// [`Self::theme_names`]) to the chart, its overlays and the series
    /// drawn there, then fit the layout to the theme's text sizes.
    /// `subplots` names the subplots that take it too: `"all"`, `"none"`
    /// or a JSON array of indices. A family name (`gallery`, `cool-slate`,
    /// `midnight`, `warm-paper`) gives the chart and every subplot in scope
    /// the member of its own kind, so subplots of other kinds take it too;
    /// a name of one kind's catalogue applies to charts of that kind
    /// (`Theme::apply_named`). Ranges, text, annotations, the layer
    /// structure and the selection stay. An unknown name, a bad list, a
    /// subplot of a kind without the name, or a draw style that cannot
    /// draw a series in scope changes nothing and errors. A chart with
    /// monotonic series keeps its layout (#271): `fit_layout`, which takes
    /// the groups' sources, fits it.
    pub fn apply_theme(&mut self, name: &str, subplots: &str) -> Result<(), JsValue> {
        self.write_theme(subplots, |config, series, scope| {
            renderer::theme::Theme::apply_named(
                name,
                config,
                series,
                scope,
                renderer::draw_style_supports,
            )
        })
    }

    /// The theme `name` resolves to on the chart (#295), as a plain object
    /// (`Theme`'s fields, colours as `{r, g, b, a}`): a family's member for
    /// the chart's kind or the kind's own theme. Edit it and pass it to
    /// [`Self::apply_custom_theme`]. An unknown name throws an `Error` with
    /// `code` `"theme_unknown_name"` and the `names` the chart takes, a
    /// name of another kind `"theme_wrong_kind"`.
    pub fn theme(&self, name: &str) -> Result<JsValue, JsValue> {
        let theme = renderer::theme::Theme::for_chart(
            name,
            self.chart_config()?,
            self.chart_series()?,
            renderer::LayerRef::Base,
        )
        .map_err(theme_error)?;
        let json = serde_json::to_string(&theme).map_err(js_err)?;
        js_sys::JSON::parse(&json)
    }

    /// Apply `theme`, an object [`Self::theme`] handed out and the host
    /// edited, to the charts of its kind (#295): the chart and every
    /// subplot of the theme's kind, or that draws no series yet; charts of
    /// another kind are left as they are. Then fit the layout as
    /// [`Self::apply_theme`] does. An object that is not a theme throws
    /// `code` `"theme_invalid"`; no chart of its kind `"theme_wrong_kind"`;
    /// a draw style that cannot draw a series it themes
    /// `"theme_draw_style"`. A refusal changes nothing.
    pub fn apply_custom_theme(&mut self, theme: JsValue) -> Result<(), JsValue> {
        let theme = theme_from_js(&theme)?;
        self.write_theme("all", |config, series, _| {
            theme.apply_to_its_kind(config, series, renderer::draw_style_supports)
        })
    }

    /// [`Self::apply_custom_theme`] to the chart and the subplots
    /// `subplots` names (`"all"`, `"none"` or a JSON array of indices, as
    /// [`Self::apply_theme`] takes them). A chart in scope of another kind
    /// throws `"theme_wrong_kind"`, an index past the subplots
    /// `"subplot_index"`.
    pub fn apply_custom_theme_to(&mut self, theme: JsValue, subplots: &str) -> Result<(), JsValue> {
        let theme = theme_from_js(&theme)?;
        self.write_theme(subplots, |config, series, scope| {
            renderer::helpers::check_theme_draw_style(&theme, config, series, scope)?;
            theme.apply(config, series, scope)
        })
    }

    /// Run `write` on a copy of the chart and its series with the scope
    /// `subplots` names, then fit the layout and adopt the copy: the one
    /// path of every theme call. A refusal throws a theme error
    /// ([`theme_error`]) and changes nothing.
    fn write_theme(
        &mut self,
        subplots: &str,
        write: impl FnOnce(
            &mut renderer::Config,
            &mut [renderer::SeriesConfig],
            SubplotScope,
        ) -> Result<(), renderer::theme::ThemeError>,
    ) -> Result<(), JsValue> {
        let indices: Vec<usize> = match subplots.trim() {
            "all" | "none" => Vec::new(),
            list => serde_json::from_str(list).map_err(|error| {
                js_err(format!(
                    "subplots {list:?} is not \"all\", \"none\" or a JSON array of indices: {error}"
                ))
            })?,
        };
        let scope = match subplots.trim() {
            "all" => SubplotScope::All,
            "none" => SubplotScope::None,
            _ => SubplotScope::Only(&indices),
        };
        let mut config = self.chart_config()?.clone();
        let mut series = self.chart_series()?.to_vec();
        write(&mut config, &mut series, scope).map_err(theme_error)?;
        // A chart with monotonic series keeps its layout: the layout fit can
        // move the plot's edges, so it is `fit_layout`, which takes the
        // groups' sources (#271).
        let config = if self.has_monotonic()? {
            config
        } else {
            let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
            renderer::layout::auto_layout(&config, &measure)
        };
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(js_err)?;
        self.replace_chart_state(config, series)
    }

    /// The names of the themes the chart takes (#291, #295), as a JSON
    /// array: the families first, then `default` and the chart kind's own
    /// themes; the families alone for a chart that draws no series yet.
    pub fn theme_names(&self) -> Result<String, JsValue> {
        use renderer::theme::{ChartKind, Theme, ThemeFamily};
        let names = ChartKind::of(
            self.chart_config()?,
            self.chart_series()?,
            renderer::LayerRef::Base,
        )
        .map_or(ThemeFamily::NAMES, Theme::names);
        serde_json::to_string(names).map_err(js_err)
    }

    /// Fill subplot `index`'s plot with a colour (JSON `{"r":1,"g":1,"b":1,
    /// "a":1}`) or leave it transparent (`null`): an inset usually covers
    /// the parent's data under it.
    pub fn set_subplot_background(&mut self, index: usize, json: &str) -> Result<(), JsValue> {
        let color: Option<Color> = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        let count = config.subplots.len();
        let subplot = config
            .subplots
            .get_mut(index)
            .ok_or_else(|| subplot_index_error(index, count))?;
        subplot.config.plot_background = color;
        self.replace_chart_config(config)
    }

    /// How subplot `index`'s text follows its cell (#292): `"Auto"` (the
    /// default) draws its titles, tick labels, legend and colourbar text at
    /// the stored sizes times `subplot_text_factor(index)`; `"Fixed"` draws
    /// the stored sizes. The stored sizes are never changed;
    /// [`Self::subplot_text_scale`] reads the mode back.
    /// JS: `chart.set_subplot_text_scale(0, "Fixed");`
    pub fn set_subplot_text_scale(&mut self, index: usize, mode: &str) -> Result<(), JsValue> {
        let text_scale = match mode {
            "Auto" => renderer::SubplotTextScale::Auto,
            "Fixed" => renderer::SubplotTextScale::Fixed,
            other => {
                return Err(js_err(format!(
                    "subplot text scale {other:?} is not \"Auto\" or \"Fixed\""
                )));
            }
        };
        let mut config = self.chart_config()?.clone();
        let count = config.subplots.len();
        let subplot = config
            .subplots
            .get_mut(index)
            .ok_or_else(|| subplot_index_error(index, count))?;
        subplot.text_scale = text_scale;
        self.replace_chart_config(config)
    }

    /// Subplot `index`'s text mode (#292, #295), as its setter
    /// [`Self::set_subplot_text_scale`] takes it: `"Auto"` or `"Fixed"`.
    pub fn subplot_text_scale(&self, index: usize) -> Result<String, JsValue> {
        let subplots = &self.chart_config()?.subplots;
        let subplot = subplots
            .get(index)
            .ok_or_else(|| subplot_index_error(index, subplots.len()))?;
        Ok(match subplot.text_scale {
            renderer::SubplotTextScale::Auto => "Auto",
            renderer::SubplotTextScale::Fixed => "Fixed",
        }
        .to_owned())
    }

    /// The factor subplot `index` draws its text with now (#292,
    /// `Config::subplot_text_factor`): 1 for a `"Fixed"` subplot or a cell
    /// as large as the parent chart, down to 0.5 for a small cell. It
    /// follows `set_subplot_area`, a frame drag and a resize of the parent
    /// chart.
    pub fn subplot_text_factor(&self, index: usize) -> Result<f32, JsValue> {
        let config = self.chart_config()?;
        if index >= config.subplots.len() {
            return Err(subplot_index_error(index, config.subplots.len()));
        }
        Ok(config.subplot_text_factor(index))
    }

    /// The subplots that are insets now (nested in another panel's plot),
    /// bottom to top, as a JSON array of indices.
    pub fn insets(&self) -> Result<String, JsValue> {
        let config = self.chart_config()?;
        let mut json = String::from("[");
        for (position, index) in config.insets().enumerate() {
            if position > 0 {
                json.push(',');
            }
            json.push_str(&index.to_string());
        }
        json.push(']');
        Ok(json)
    }
}
