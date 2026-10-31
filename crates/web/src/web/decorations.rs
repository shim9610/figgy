//! Axes, titles, legend, colorbar, annotations, decoration bounds and
//! auto-fit.

use super::*;

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
        let mut chart = Chart::new(self.chart_config().clone());
        chart
            .auto_fit_x(self.renderer.pool(), column, padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
        let mut chart = Chart::new(self.chart_config().clone());
        chart
            .auto_fit_y(self.renderer.pool(), column, padding)
            .map_err(js_err)?;
        self.replace_chart_config(chart.config().clone())?;
        Ok(())
    }

    /// Fit overlay layer `layer`'s Y axis to a column's cached statistics
    /// with proportional padding, the overlay counterpart of `auto_fit_y`.
    /// A layer index past the overlay list is an error.
    /// JS: `chart.auto_fit_overlay_y(0, "pressure", 0.05)`
    pub fn auto_fit_overlay_y(
        &mut self,
        layer: usize,
        column: &str,
        padding: f64,
    ) -> Result<(), JsValue> {
        if layer >= self.chart_config().overlays.len() {
            return Err(js_err("overlay layer index is out of range"));
        }
        let mut chart = Chart::new(self.chart_config().clone());
        chart
            .auto_fit_overlay_y_union(self.renderer.pool(), layer, &[column], padding)
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
        if index >= self.chart_config().subplots.len() {
            return Err(js_err("subplot index is out of range"));
        }
        let ids: Vec<&str> = columns.iter().map(String::as_str).collect();
        let mut chart = Chart::new(self.chart_config().clone());
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
    /// `SubplotOverlay(i, n)` fit that overlay's Y axis only. An axis fits
    /// every series drawn against it (`Config::layer_of_series`). A linked
    /// axis (`link_x`, `link_y`) is one axis shared by its link group: it
    /// fits the series of every chart in the group and the range moves
    /// the whole group, whichever member is active. X and Y links are
    /// followed separately. An overlay shares its chart's X axis and has
    /// its own Y axis, so its series count towards that X axis and not
    /// towards `left_y`. An axis that is not linked fits its own chart's
    /// series only; to fit every layer, set each layer active in turn and
    /// call this again.
    ///
    /// Each series contributes its exact original GPU primitive domain:
    /// valid adjacent segments for line-only mode, paired finite points
    /// for point-bearing modes, and enabled error endpoints. The reduction
    /// is compiled on first fit (`createComputePipelineAsync` on wasm)
    /// and cached by the normalized mode plus role-specific column
    /// revisions. Matrix-backed series use the same engine to return the
    /// exact cell/sample-lattice bounds; only histograms remain a metadata
    /// union because their uploaded edge column already is their geometry.
    /// Series registration itself does not submit GPU work.
    pub async fn auto_fit_all(&mut self, padding: f64) -> Result<(), JsValue> {
        if self.request_stream_auto_fit(padding)? {
            self.request_host_redraw();
            return Ok(());
        }
        // A ternary chart's axes span the whole triangle, not a data extent.
        let ternary = {
            let config = self.chart_config();
            match self.renderer.active_layer(self.chart_id).map_err(js_err)? {
                renderer::LayerRef::Subplot(index)
                | renderer::LayerRef::SubplotOverlay(index, _) => config
                    .subplots
                    .get(index)
                    .is_some_and(|subplot| subplot.config.ternary.is_some()),
                _ => config.ternary.is_some(),
            }
        };
        if ternary {
            return Ok(());
        }
        let series = self.chart_series().to_vec();
        if series
            .iter()
            .any(|cfg| series_extent_mode(&cfg.render_type).is_some())
        {
            self.renderer
                .ensure_errorbar_extent_engine()
                .await
                .map_err(js_err)?;
        }
        // The annotation layer draws on the base axes.
        let layer = match self.renderer.active_layer(self.chart_id).map_err(js_err)? {
            renderer::LayerRef::Annotations => renderer::LayerRef::Base,
            layer => layer,
        };
        let token = self
            .renderer
            .begin_fit_commit(self.chart_id)
            .map_err(js_err)?;
        let prepared = self.prepare_series_extent_cache(&series, &self.column_revisions, true)?;
        // The cache keeps every series' extent job; only the series drawn
        // against the fitted axes take part in this fit.
        let config = self.chart_config();
        let mut jobs = Vec::new();
        jobs.try_reserve(series.len()).map_err(js_err)?;
        jobs.extend(series.iter().filter_map(|cfg| {
            let (x, y) = config.series_fit_roles(layer, &cfg.series_id);
            if !(x || y) {
                return None;
            }
            let request = crate::series_extent_request_from(&self.column_revisions, cfg)?;
            Some((
                prepared.extents.get(&request.key).cloned()?,
                cfg.series_id.as_str(),
                x,
                y,
            ))
        }));
        self.publish_series_extent_cache(prepared);

        let mut x_ext = FitExtent::EMPTY;
        let mut y_ext = FitExtent::EMPTY;
        for (job, series_id, x, y) in jobs {
            let extent = job.wait().await.map_err(js_err)?;
            if let Some(extent) = extent {
                // A line offset member fits where it is drawn.
                let (extent_x, extent_y) = Chart::line_offset_fit_extents(
                    self.chart_config(),
                    series_id,
                    extent.x,
                    extent.y,
                );
                if x {
                    x_ext.union(&extent_x);
                }
                if y {
                    y_ext.union(&extent_y);
                }
            }
        }
        let scalar_extent = |value: f64| FitExtent {
            min: value,
            max: value,
            min_positive: (value > 0.0).then_some(value),
        };
        for cfg in &series {
            let (x, y) = self.chart_config().series_fit_roles(layer, &cfg.series_id);
            match &cfg.render_type {
                DataRenderType::Histogram { bar } if x || y => {
                    let (edge_column, count_column) =
                        crate::histogram_fit_columns(cfg, &bar.orientation);
                    let edge =
                        Chart::slot_extent(self.renderer.pool(), edge_column).map_err(js_err)?;
                    let mut count =
                        Chart::slot_extent(self.renderer.pool(), count_column).map_err(js_err)?;
                    count.union(&scalar_extent(bar.baseline));
                    let (x_part, y_part) = if matches!(
                        &bar.orientation,
                        renderer::data_config::BarOrientation::Vertical
                    ) {
                        (edge, count)
                    } else {
                        (count, edge)
                    };
                    if x {
                        x_ext.union(&x_part);
                    }
                    if y {
                        y_ext.union(&y_part);
                    }
                }
                _ => {}
            }
        }
        if !self.alive.get() {
            return Err(js_err("chart was dropped while auto_fit_all was pending"));
        }
        if layer == renderer::LayerRef::Base {
            self.renderer
                .commit_auto_fit_all_if_current(&token, &x_ext, &y_ext, padding)
                .map_err(js_err)?;
        } else {
            // The same staleness rule as the base commit: a range, scale,
            // geometry or column change during the wait rejects the fit.
            if self
                .renderer
                .begin_fit_commit(self.chart_id)
                .map_err(js_err)?
                != token
            {
                return Err(js_err(
                    "axis range/scale, ordered fit geometry, or source column content changed",
                ));
            }
            let mut chart = Chart::new(self.chart_config().clone());
            chart.auto_fit_layer_extent(layer, &x_ext, &y_ext, padding);
            self.replace_chart_config(chart.config().clone())?;
        }
        // A 3D chart's z axis fits to its height columns' statistics.
        let config = self.chart_config();
        let three_d = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .is_some_and(|subplot| subplot.config.view_3d.is_some())
            }
            _ => config.view_3d.is_some(),
        };
        if three_d {
            let mut chart = Chart::new(config.clone());
            chart
                .auto_fit_view_3d_z(self.renderer.pool(), layer, &series, padding)
                .map_err(js_err)?;
            self.replace_chart_config(chart.config().clone())?;
        }
        self.request_host_redraw();
        Ok(())
    }

    /// Fit the shared z axis to every column the colourbar maps: matrix
    /// value columns, scatter `point_color_map` columns, vector colour
    /// maps and streamline colour columns. A chart without a colourbar is left unchanged.
    pub fn auto_fit_colorbar(&mut self, padding: f64) -> Result<(), JsValue> {
        let mut z_extent = FitExtent::EMPTY;
        for cfg in self.chart_series() {
            let mut union = |id: &str| -> Result<(), JsValue> {
                z_extent.union(&Chart::slot_extent(self.renderer.pool(), id).map_err(js_err)?);
                Ok(())
            };
            match &cfg.render_type {
                DataRenderType::Heatmap { matrix, .. }
                | DataRenderType::Contour { matrix, .. }
                | DataRenderType::HeatmapContour { matrix, .. } => {
                    for id in &matrix.columns {
                        union(id)?;
                    }
                }
                DataRenderType::Scatter { scatter }
                | DataRenderType::ScatterLine { scatter, .. }
                | DataRenderType::ScatterErrorbarX { scatter, .. }
                | DataRenderType::ScatterErrorbarY { scatter, .. }
                | DataRenderType::ScatterErrorbarXY { scatter, .. }
                | DataRenderType::LineScatterErrorbarX { scatter, .. }
                | DataRenderType::LineScatterErrorbarY { scatter, .. }
                | DataRenderType::LineScatterErrorbarXY { scatter, .. } => {
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
                DataRenderType::Line { .. }
                | DataRenderType::Histogram { .. }
                | DataRenderType::CategoryBars { .. }
                | DataRenderType::BoxPlot { .. }
                | DataRenderType::RangeBars { .. }
                | DataRenderType::Radar { .. }
                | DataRenderType::Ternary { scatter: None, .. } => {}
            }
        }
        let mut chart = Chart::new(self.chart_config().clone());
        chart.auto_fit_colorbar_extent(&z_extent, padding);
        self.replace_chart_config(chart.config().clone())?;
        self.request_host_redraw();
        Ok(())
    }

    // ---- titles ----

    /// Set plain title text and grow its band if needed. For a text-only
    /// edit that keeps the exact margin, use `get_config` / `set_config`
    /// and change only `chart_title.text`.
    pub fn set_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config().clone();
        set_plain_chart_title(&mut config, text);
        self.replace_chart_config(config)
    }

    pub fn set_x_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config().clone();
        config.bottom_x.title_option.text.segments = rich_segments_from_text(text);
        self.replace_chart_config(config)
    }

    pub fn set_y_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config().clone();
        config.left_y.title_option.text.segments = rich_segments_from_text(text);
        self.replace_chart_config(config)
    }

    /// Set the colourbar title text and make it visible. An empty string
    /// hides it. The full rich-text/title offsets remain available through
    /// `set_colorbar_axis` or the chart Config SSoT.
    pub fn set_colorbar_title(&mut self, text: &str) -> Result<(), JsValue> {
        let mut config = self.chart_config().clone();
        let bar = config
            .colorbar
            .as_mut()
            .ok_or_else(|| js_err("colorbar title needs Config.colorbar"))?;
        bar.axis.title_option.text.segments = rich_segments_from_text(text);
        bar.axis.title_option.visible = !text.is_empty();
        self.replace_chart_config(config)
    }

    /// Replace the colourbar's z-axis options as one SSoT value. This is
    /// the focused edit boundary for tick direction/style/length, value
    /// direction, tick-label style/offsets, and title options.
    pub fn set_colorbar_axis(&mut self, json: &str) -> Result<(), JsValue> {
        let axis: renderer::config::AxisOptions = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config().clone();
        let bar = config
            .colorbar
            .as_mut()
            .ok_or_else(|| js_err("colorbar axis needs Config.colorbar"))?;
        bar.axis = axis;
        self.replace_chart_config(config)
    }

    // ---- presets ----

    /// Apply an axis frame preset to all four axes (decoration-only).
    pub fn apply_axis_preset(&mut self, preset: AxisPreset) -> Result<(), JsValue> {
        let p: renderer::AxisPreset = preset.into();
        let mut config = self.chart_config().clone();
        config.apply_axis_preset(p);
        self.replace_chart_config(config)
    }

    /// Switch the series color rotation: recolors every series in order,
    /// rebuilds their GPU styles, and keeps legend swatches in sync.
    pub fn apply_color_cycle(&mut self, cycle: ColorCycle) -> Result<(), JsValue> {
        let cycle = renderer::ColorCycle::from(cycle);
        let mut series = self.chart_series().to_vec();
        for (i, cfg) in series.iter_mut().enumerate() {
            cycle.apply_to_series(cfg, i);
        }
        let (scale, _) = self.display_scale_and_panel();
        let mut styles = Vec::new();
        styles.try_reserve(series.len()).map_err(js_err)?;
        for cfg in &series {
            styles.push(
                self.renderer
                    .create_style_for_series_scaled(cfg, scale)
                    .map_err(js_err)?,
            );
        }
        let mut config = self.chart_config().clone();
        renderer::config::update_legend_symbols_preserving_text(
            &mut config.legend.content,
            &series,
            config.colorbar.as_ref().map(|bar| &bar.colormap),
        );
        self.replace_chart_state(config, series)?;
        self.cycle = cycle;
        self.color_seq = styles.len();
        self.styles = styles;
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
        let mut config = self.chart_config().clone();
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
        let items = &self.chart_config().annotations.items;
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
            && i >= self.chart_config().annotations.items.len()
        {
            return Err(js_err("unknown annotation"));
        }
        if index.is_some() && self.chart_selection().is_some() {
            self.renderer
                .set_chart_selection(self.chart_id, None)
                .map_err(js_err)?;
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
        let id = self.annotation_hit_id(index as usize).ok()?;
        self.view
            .annotation_map()
            .hit_resize_handle(id, [x, y])
            .map(|h| format!("{h:?}"))
    }

    /// The vertex handle (`"v<i>"`) of a polygon annotation under a
    /// point, or none: a polygon's handles are its vertices.
    pub fn annotation_vertex(&self, index: u32, x: f32, y: f32) -> Option<String> {
        let id = self.annotation_hit_id(index as usize).ok()?;
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
            let mut config = self.chart_config().clone();
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
        let mut config = self.chart_config().clone();
        config.annotations.items.push(item);
        let index = config.annotations.items.len() - 1;
        self.replace_chart_config(config)?;
        Ok(index as u32)
    }

    /// Replace the annotation at `index` with an `Annotation` JSON value.
    pub fn set_annotation(&mut self, index: u32, json: &str) -> Result<(), JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config().clone();
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
        let mut config = self.chart_config().clone();
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
        let mut config = self.chart_config().clone();
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
        let (config, _, _) = self.display_config();
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
