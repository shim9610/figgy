//! Hover, picking, selection and pointer interaction.

use super::*;

/// The point style of a series that draws markers, whichever variant
/// carries it.
pub(super) fn scatter_style(rt: &DataRenderType) -> Option<&renderer::DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
        DataRenderType::Line { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => None,
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// Set the layer that receives pointer interaction: `"Base"`,
    /// `{ "Overlay": index }`, `"Annotations"`, `{ "Subplot": index }` or
    /// `{ "SubplotOverlay": [subplot, overlay] }` as JSON. The hit map is
    /// rebuilt for that layer (`HitMap::for_active_layer`) and the
    /// element selection is cleared when the layer changes. An overlay
    /// index past the overlay list is stored as `"Base"`; the stored
    /// layer is returned as JSON. Drawn pixels never depend on it.
    /// JS: `chart.set_active_layer(JSON.stringify({ Overlay: 0 }))`
    pub fn set_active_layer(&mut self, json: &str) -> Result<String, JsValue> {
        let layer: renderer::LayerRef = serde_json::from_str(json).map_err(js_err)?;
        let stored = self
            .renderer
            .set_active_layer(self.chart_id, layer)
            .map_err(js_err)?;
        self.sync_hitmap();
        serde_json::to_string(&stored).map_err(js_err)
    }

    /// The layer that receives pointer interaction, as JSON, with the
    /// out-of-range fallback applied.
    /// JS: `JSON.parse(chart.active_layer())`
    pub fn active_layer(&self) -> Result<String, JsValue> {
        let layer = self.renderer.active_layer(self.chart_id).map_err(js_err)?;
        serde_json::to_string(&layer).map_err(js_err)
    }

    // ---- pointer interaction (coordinates in canvas pixels) ----

    /// Hit-test the chart chrome at canvas pixel `(x, y)` — returns the
    /// topmost element's stable id (`"data_area"`, `"axis_bottom"`,
    /// `"tick_labels_left"`, `"axis_title_left"`, `"colorbar"`,
    /// `"colorbar_axis"`, `"colorbar_tick_labels"`,
    /// `"colorbar_title"`, …) or `null`. Pure geometry, no selection state
    /// change: the renderer's own layout answers, so hosts don't have to
    /// re-derive box positions for hover cursors / context UI. The set of
    /// registered elements follows the active layer: while an overlay is
    /// active its elements answer as `"overlay_<n>:axis_right"`,
    /// `"overlay_<n>:tick_labels_right"`, `"overlay_<n>:axis_title_right"`
    /// and `"overlay_<n>:legend"`, and the base Y axes do not answer.
    /// While the annotation layer is active (and shown) its drawn items
    /// sit on top and answer as `"annotation_<n>"`; on every other layer
    /// they are not registered and clicks pass through them.
    pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
        let (display_config, _, _) = self.display_config();
        match self
            .view
            .hit_test_active_layer(&display_config, &self.hitmap, [x, y])?
        {
            LayerHit::Annotation { index, .. } => {
                Some(renderer::DecorationId::Annotation(index).element_id())
            }
            LayerHit::Element(id) => self.hitmap.get(id).map(|el| el.element_id()),
        }
    }

    // Copy only the selected scalar pairs out of the resident GPU pool.
    // Registered source arrays are not retained or reconstructed on CPU.
    pub(super) async fn read_selection_samples(
        &self,
        samples: &[(String, usize)],
    ) -> Result<Vec<f64>, JsValue> {
        let offsets = samples
            .iter()
            .map(|(column, index)| {
                let handle = self
                    .renderer
                    .pool()
                    .handle_for(column)
                    .ok_or_else(|| js_err("selected column no longer exists"))?;
                if *index >= handle.len_values {
                    return Err(js_err("selected index is outside the column"));
                }
                Ok(handle.offset + *index as u64 * 8)
            })
            .collect::<Result<Vec<_>, JsValue>>()?;
        let buffer = self
            .renderer
            .device()
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("figgy selected values"),
                size: offsets.len() as u64 * 8,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        let mut encoder = self
            .renderer
            .device()
            .create_command_encoder(&Default::default());
        for (index, offset) in offsets.iter().enumerate() {
            encoder.copy_buffer_to_buffer(
                self.renderer.pool().buffer(),
                *offset,
                &buffer,
                index as u64 * 8,
                8,
            );
        }
        self.renderer.queue().submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (sender, receiver) = futures_channel::oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        receiver.await.map_err(js_err)?.map_err(js_err)?;
        let mapped = slice.get_mapped_range().map_err(js_err)?;
        let values = mapped
            .chunks_exact(8)
            .map(|pair| {
                f32::from_le_bytes(pair[..4].try_into().unwrap()) as f64
                    + f32::from_le_bytes(pair[4..].try_into().unwrap()) as f64
            })
            .collect();
        drop(mapped);
        buffer.unmap();
        Ok(values)
    }

    /// Inspect the first resident typed selection as JSON, or `"null"` when
    /// none is selected. Only selected GPU samples are read back. Histograms
    /// expose `bin_start`, `bin_end`, `value`, and `orientation`; the value
    /// is the stored height (also valid for weighted/density histograms).
    /// Point and matrix selections expose their coordinates and cell value.
    pub async fn selection_details(&self) -> Result<String, JsValue> {
        use renderer::config::PickedDataRef;
        let Some(reference) = self
            .chart_config()
            .picked_data
            .as_ref()
            .and_then(|selection| selection.refs.first())
            .cloned()
        else {
            return Ok("null".into());
        };
        let series = self
            .chart_series()
            .iter()
            .find(|series| series.series_id == reference.series_id())
            .cloned()
            .ok_or_else(|| js_err("selected series no longer exists"))?;
        let mut result = serde_json::json!({"identity": reference, "series_id": series.series_id});
        match &reference {
            PickedDataRef::Point { point_index, .. } => {
                // A colour- or size-mapped point also reports the value its
                // colour or radius was derived from, read back from the
                // same GPU column the draw used.
                let scatter = scatter_style(&series.render_type);
                let color_column = scatter
                    .and_then(|scatter| scatter.point_color_map.as_ref())
                    .map(|map| map.column.clone());
                let size_column = scatter
                    .and_then(|scatter| scatter.point_size_map.as_ref())
                    .map(|map| map.column.clone());
                let mut samples = vec![
                    (series.x_column, *point_index),
                    (series.y_column, *point_index),
                ];
                if let Some(column) = color_column.clone() {
                    samples.push((column, *point_index));
                }
                if let Some(column) = size_column.clone() {
                    samples.push((column, *point_index));
                }
                // A point vector reports its components and, when colour
                // mapped, the value its colour came from.
                let vector = match vector_columns(&series.render_type) {
                    Some(VectorColumns::Points { u, v, color }) => {
                        samples.push((u.to_owned(), *point_index));
                        samples.push((v.to_owned(), *point_index));
                        if let Some(color) = color {
                            samples.push((color.to_owned(), *point_index));
                        }
                        Some(color.is_some())
                    }
                    _ => None,
                };
                let values = self.read_selection_samples(&samples).await?;
                result["index"] = serde_json::json!(point_index);
                result["x"] = serde_json::json!(values[0]);
                result["y"] = serde_json::json!(values[1]);
                let mut next = 2;
                if color_column.is_some() {
                    result["color_value"] = serde_json::json!(values[next]);
                    next += 1;
                }
                if size_column.is_some() {
                    result["size_value"] = serde_json::json!(values[next]);
                    next += 1;
                }
                if let Some(colour_mapped) = vector {
                    result["u"] = serde_json::json!(values[next]);
                    result["v"] = serde_json::json!(values[next + 1]);
                    if colour_mapped {
                        result["color_value"] = serde_json::json!(values[next + 2]);
                    }
                }
            }
            PickedDataRef::HistogramBin { bin_index, .. } => {
                let renderer::DataRenderType::Histogram { bar } = &series.render_type else {
                    return Err(js_err("selected series is not a histogram"));
                };
                let (edges, heights) = match bar.orientation {
                    renderer::data_config::BarOrientation::Vertical => {
                        (&series.x_column, &series.y_column)
                    }
                    renderer::data_config::BarOrientation::Horizontal => {
                        (&series.y_column, &series.x_column)
                    }
                };
                let values = self
                    .read_selection_samples(&[
                        (edges.clone(), *bin_index),
                        (edges.clone(), bin_index + 1),
                        (heights.clone(), *bin_index),
                    ])
                    .await?;
                result["index"] = serde_json::json!(bin_index);
                result["bin_start"] = serde_json::json!(values[0]);
                result["bin_end"] = serde_json::json!(values[1]);
                result["value"] = serde_json::json!(values[2]);
                result["orientation"] = serde_json::json!(bar.orientation);
            }
            PickedDataRef::MatrixCell {
                x_index, y_index, ..
            } => {
                use renderer::data_config::{GridLayout, MatrixOrientation};
                // A field's matrix, or a grid vector's `u` grid with its `v`
                // grid and colour grid beside it.
                let (matrix, vector) = match &series.render_type {
                    DataRenderType::Heatmap { matrix, .. }
                    | DataRenderType::Contour { matrix, .. }
                    | DataRenderType::HeatmapContour { matrix, .. } => (matrix, None),
                    rt => match vector_columns(rt) {
                        Some(VectorColumns::Grid { u, v, color }) => (u, Some((v, color))),
                        _ => return Err(js_err("selected series is not a grid")),
                    },
                };
                let across_x = matches!(matrix.orientation, MatrixOrientation::ColumnsAreY);
                let column_index = if across_x { *y_index } else { *x_index };
                let sample_index = if across_x { *x_index } else { *y_index };
                let grid_column = |grid: &renderer::data_config::MatrixRef| {
                    grid.columns
                        .get(column_index)
                        .cloned()
                        .ok_or_else(|| js_err("selected cell no longer exists"))
                };
                let mut samples = vec![
                    (series.x_column.clone(), *x_index),
                    (series.y_column.clone(), *y_index),
                    (grid_column(matrix)?, sample_index),
                ];
                let edges = matches!(matrix.grid_layout, GridLayout::Edges);
                if edges {
                    samples.extend([
                        (series.x_column.clone(), x_index + 1),
                        (series.y_column.clone(), y_index + 1),
                    ]);
                }
                if let Some((v, color)) = vector {
                    samples.push((grid_column(v)?, sample_index));
                    if let Some(color) = color {
                        samples.push((grid_column(color)?, sample_index));
                    }
                }
                let values = self.read_selection_samples(&samples).await?;
                result["x_index"] = serde_json::json!(x_index);
                result["y_index"] = serde_json::json!(y_index);
                result["x"] = serde_json::json!(if edges {
                    (values[0] + values[3]) * 0.5
                } else {
                    values[0]
                });
                result["y"] = serde_json::json!(if edges {
                    (values[1] + values[4]) * 0.5
                } else {
                    values[1]
                });
                let mut next = if edges { 5 } else { 3 };
                match vector {
                    None => result["value"] = serde_json::json!(values[2]),
                    Some((_, color)) => {
                        result["u"] = serde_json::json!(values[2]);
                        result["v"] = serde_json::json!(values[next]);
                        next += 1;
                        if color.is_some() {
                            result["color_value"] = serde_json::json!(values[next]);
                        }
                    }
                }
            }
            _ => return Ok("null".into()),
        }
        serde_json::to_string(&result).map_err(js_err)
    }

    /// Move the resident typed data selection using source indices.
    pub fn step_selection(&mut self, dx: i32, dy: i32) -> Result<bool, JsValue> {
        let mut cfg = self.chart_config().clone();
        let Some(reference) = cfg.picked_data.as_mut().and_then(|s| s.refs.first_mut()) else {
            return Ok(false);
        };
        let Some(position) = self
            .chart_series()
            .iter()
            .position(|s| s.series_id == reference.series_id())
        else {
            return Ok(false);
        };
        let shift = |index: usize, delta: i32, count: usize| {
            index
                .saturating_add_signed(delta as isize)
                .min(count.saturating_sub(1))
        };
        match reference {
            renderer::config::PickedDataRef::Point {
                series_id,
                source_id,
                point_index,
            } => {
                // Point-bearing series: lines, markers and point vectors,
                // whose anchors pick and highlight like markers.
                let points = self
                    .chart_series()
                    .iter()
                    .filter(|s| {
                        let style = serde_json::to_value(&s.render_type).unwrap();
                        style
                            .as_object()
                            .unwrap()
                            .values()
                            .next()
                            .is_some_and(|v| v["line"].is_object() || v["scatter"].is_object())
                            || matches!(
                                vector_columns(&s.render_type),
                                Some(VectorColumns::Points { .. })
                            )
                    })
                    .collect::<Vec<_>>();
                let Some(index) = points.iter().position(|s| s.series_id == *series_id) else {
                    return Ok(false);
                };
                let series = points[shift(index, dy, points.len())];
                let mut count = self.columns[&series.x_column].min(self.columns[&series.y_column]);
                // A point vector draws only where all four columns have a value.
                if let Some(VectorColumns::Points { u, v, .. }) =
                    vector_columns(&series.render_type)
                {
                    count = count.min(self.columns[u]).min(self.columns[v]);
                }
                *series_id = series.series_id.clone();
                *source_id = series.source_id.clone();
                *point_index = shift(*point_index, dx, count);
            }
            renderer::config::PickedDataRef::HistogramBin {
                series_id,
                source_id,
                bin_index,
            } => {
                let renderer::DataRenderType::Histogram { bar } =
                    &self.chart_series()[position].render_type
                else {
                    return Ok(false);
                };
                let horizontal = matches!(
                    bar.orientation,
                    renderer::data_config::BarOrientation::Horizontal
                );
                let histograms = self
                    .chart_series()
                    .iter()
                    .filter(|s| {
                        matches!(&s.render_type,
                            renderer::DataRenderType::Histogram { bar: other }
                            if other.orientation == bar.orientation)
                    })
                    .collect::<Vec<_>>();
                let index = histograms
                    .iter()
                    .position(|s| s.series_id == *series_id)
                    .unwrap();
                let (bin_delta, series_delta) = if horizontal { (dy, dx) } else { (dx, dy) };
                let series = histograms[shift(index, series_delta, histograms.len())];
                let (edges, values) = if horizontal {
                    (&series.y_column, &series.x_column)
                } else {
                    (&series.x_column, &series.y_column)
                };
                let count = self.columns[edges]
                    .saturating_sub(1)
                    .min(self.columns[values]);
                if count == 0 {
                    return Ok(false);
                }
                *series_id = series.series_id.clone();
                *source_id = series.source_id.clone();
                *bin_index = shift(*bin_index, bin_delta, count);
            }
            renderer::config::PickedDataRef::MatrixCell {
                x_index, y_index, ..
            } => {
                use renderer::data_config::{GridLayout, MatrixOrientation};
                let series = &self.chart_series()[position];
                // A field steps over its matrix; a grid vector over its
                // `u` grid, which every other grid of the series matches.
                let matrix = match &series.render_type {
                    DataRenderType::Heatmap { matrix, .. }
                    | DataRenderType::Contour { matrix, .. }
                    | DataRenderType::HeatmapContour { matrix, .. } => matrix,
                    rt => match vector_columns(rt) {
                        Some(VectorColumns::Grid { u, .. }) => u,
                        _ => return Err(js_err("selected matrix no longer exists")),
                    },
                };
                let edges = usize::from(matches!(matrix.grid_layout, GridLayout::Edges));
                let depth = matrix
                    .columns
                    .iter()
                    .filter_map(|c| self.columns.get(c.as_str()))
                    .copied()
                    .min()
                    .unwrap_or(0);
                let (nx, ny) = if matches!(matrix.orientation, MatrixOrientation::ColumnsAreY) {
                    (depth, matrix.columns.len())
                } else {
                    (matrix.columns.len(), depth)
                };
                *x_index = shift(
                    *x_index,
                    dx,
                    nx.min(self.columns[&series.x_column].saturating_sub(edges)),
                );
                *y_index = shift(
                    *y_index,
                    dy,
                    ny.min(self.columns[&series.y_column].saturating_sub(edges)),
                );
            }
            _ => return Ok(false),
        }
        if cfg == *self.chart_config() {
            return Ok(false);
        }
        self.replace_chart_config(cfg)?;
        Ok(true)
    }

    pub(super) async fn prepare_gpu_picking(&mut self) -> Result<(), JsValue> {
        self.renderer
            .enable_gpu_picking_async()
            .await
            .map_err(js_err)?;
        self.renderer
            .prepare_gpu_picking_for_chart(self.chart_id)
            .map_err(js_err)
    }

    /// Compile the exact GPU picker and prepare it for the current chart.
    /// Repeated calls, including retries, reuse renderer-owned state.
    pub async fn prewarm_gpu_picking(&mut self) -> Result<(), JsValue> {
        self.prepare_gpu_picking().await
    }

    /// Pick the nearest visible data primitive to canvas pixel `(x, y)`.
    /// Scatter hits use the visible marker size, including per-point style
    /// mapping; line strokes snap to the nearest endpoint data point on
    /// the hit segment. Errorbar stems/caps are not pick targets.
    /// Resolves to a JSON string
    /// `{ source_id, series_id, point_index, distance_px }`, or
    /// `undefined` when no visible primitive is within `max_distance_px`.
    /// The `<figgy-chart>` facade parses the string and normalizes
    /// `undefined` to `null`.
    pub async fn pick_point(
        &mut self,
        x: f32,
        y: f32,
        max_distance_px: f32,
    ) -> Result<JsValue, JsValue> {
        if self.renderer.is_streaming_chart(self.chart_id) {
            let Some(picked) = self
                .renderer
                .pick_chart_view_cache_at(self.chart_id, [x, y], max_distance_px)
                .await
                .map_err(js_err)?
            else {
                return Ok(JsValue::UNDEFINED);
            };
            let json = crate::picked_point_json_string(&picked).map_err(js_err)?;
            return Ok(JsValue::from_str(&json));
        }
        self.prepare_gpu_picking().await?;
        let ticket = self
            .renderer
            .pick_chart_at(self.chart_id, [x, y], max_distance_px)
            .map_err(js_err)?;
        let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
            return Ok(JsValue::UNDEFINED);
        };
        let json = crate::picked_point_json_string(&picked).map_err(js_err)?;
        Ok(JsValue::from_str(&json))
    }

    /// Move among source rows represented in the completed packed view.
    /// Returns only the original source index; no source or GPU readback.
    pub fn next_view_point_index(
        &self,
        source_id: Option<String>,
        series_id: String,
        current: f64,
        forward: bool,
    ) -> Result<JsValue, JsValue> {
        let current = js_safe_u64(current, "current point index")?;
        let current =
            usize::try_from(current).map_err(|_| js_err("point index exceeds platform size"))?;
        Ok(self
            .renderer
            .next_view_point_index(
                self.chart_id,
                source_id.as_deref(),
                &series_id,
                current,
                forward,
            )
            .map_or(JsValue::UNDEFINED, |index| JsValue::from_f64(index as f64)))
    }

    /// Pick the nearest visible data primitive at canvas pixel `(x, y)`;
    /// equal-distance hits follow chart paint order.
    /// Unlike `pick_point`, this includes histogram bins, canonical heatmap
    /// cells and contour levels. The tagged result contains only stable
    /// source/series indices plus `distance_px`; values and geometry stay
    /// in the renderer's GPU-backed SSoT. Resolves to `undefined` on miss.
    pub async fn pick_data(
        &mut self,
        x: f32,
        y: f32,
        max_distance_px: f32,
    ) -> Result<JsValue, JsValue> {
        if self.renderer.is_streaming_chart(self.chart_id) {
            let Some(picked) = self
                .renderer
                .pick_chart_view_cache_at(self.chart_id, [x, y], max_distance_px)
                .await
                .map_err(js_err)?
            else {
                return Ok(JsValue::UNDEFINED);
            };
            let tagged = renderer::PickedData::Point {
                source_id: picked.source_id,
                series_id: picked.series_id,
                point_index: picked.point_index,
                distance_px: picked.distance_px,
            };
            let json = crate::picked_data_json_string(&tagged).map_err(js_err)?;
            return Ok(JsValue::from_str(&json));
        }
        self.prepare_gpu_picking().await?;
        let ticket = self
            .renderer
            .pick_chart_data_at(self.chart_id, [x, y], max_distance_px)
            .map_err(js_err)?;
        let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
            return Ok(JsValue::UNDEFINED);
        };
        let json = crate::picked_data_json_string(&picked).map_err(js_err)?;
        Ok(JsValue::from_str(&json))
    }

    /// Replace the picked-point overlay config. Passing JSON `null`
    /// clears it.
    pub fn set_picked_points(&mut self, json: &str) -> Result<(), JsValue> {
        let picked: Option<renderer::config::PickedPointsConfig> =
            serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config().clone();
        config.picked_points = picked;
        self.replace_chart_config(config)
    }

    /// Replace the typed data-selection overlay config. Passing JSON
    /// `null` clears it. This is independent of the legacy point-only
    /// overlay so existing hosts keep their current behavior.
    pub fn set_picked_data(&mut self, json: &str) -> Result<(), JsValue> {
        let picked: Option<renderer::DataSelectionsConfig> =
            serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config().clone();
        config.picked_data = picked;
        self.replace_chart_config(config)
    }

    /// Pointer press. Returns `true` while something is selected.
    /// The host can mirror that state in its own UI.
    ///
    /// The press follows the active layer's route: while the annotation
    /// layer is active (and shown), the selected annotation's handles win
    /// first, then a drawn item under the point is selected (clearing
    /// the element selection) and dragged by `on_move`; a press that
    /// misses every item falls through to the shared elements below and
    /// clears the annotation selection. On every other layer the items
    /// are not part of the route.
    pub fn on_press(&mut self, x: f32, y: f32) -> Result<bool, JsValue> {
        let (display_config, _, _) = self.display_config();
        let routes_annotations = renderer::AnnotationMap::receives_pointer(
            display_config.active_layer(),
            &display_config.annotations,
        );
        if routes_annotations
            && let Some(index) = self.selected_annotation
            && let Ok(id) = self.annotation_hit_id(index)
            && let Some(handle) = self.view.annotation_map().hit_resize_handle(id, [x, y])
        {
            self.resizing = Some(handle);
            self.dragging = false;
            self.annotation_pointer = true;
            return Ok(true);
        }
        if let Some(LayerHit::Annotation { index, .. }) =
            self.view
                .hit_test_active_layer(&display_config, &self.hitmap, [x, y])
        {
            if self.chart_selection().is_some() {
                self.renderer
                    .set_chart_selection(self.chart_id, None)
                    .map_err(js_err)?;
            }
            self.set_selected_annotation(Some(index));
            self.resizing = None;
            self.dragging = true;
            self.annotation_pointer = true;
            return Ok(true);
        }
        self.annotation_pointer = false;
        let selected = self.chart_selection();
        // Resize handles on the selected element win over hit-testing.
        if let Some(id) = selected
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
            return Ok(true);
        }

        let new_sel = self.hitmap.hit_test(
            &display_config,
            &CpuTextMeasure::for_style(&display_config.draw_style),
            x,
            y,
        );
        let dragging = new_sel.is_some_and(|id| {
            self.hitmap
                .get(id)
                .is_some_and(|el| el.as_draggable().is_some())
        });
        if new_sel != selected {
            self.renderer
                .set_chart_selection(self.chart_id, new_sel)
                .map_err(js_err)?;
        }
        if new_sel.is_some() || routes_annotations {
            self.set_selected_annotation(None);
        }
        self.resizing = None;
        self.dragging = dragging;
        self.drag_pointer = self.display_point_to_document(x, y);
        Ok(new_sel.is_some())
    }

    /// Pointer move with frame delta — drags or resizes the selection.
    /// After a press on an annotation the delta moves that item (or the
    /// pressed handle) through the frame's anchor space.
    pub fn on_move(&mut self, dx: f32, dy: f32) -> Result<(), JsValue> {
        if self.annotation_pointer {
            if let Some(index) = self.selected_annotation
                && (self.dragging || self.resizing.is_some())
                && dx.is_finite()
                && dy.is_finite()
            {
                self.move_annotation(index, dx, dy, self.resizing)?;
            }
            return Ok(());
        }
        let Some(id) = self.chart_selection() else {
            return Ok(());
        };
        let (dx, dy) = self.display_delta_to_document(dx, dy);
        let mut config = self.chart_config().clone();
        if let Some(handle) = self.resizing {
            if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                && rz.resize_by(&mut config, handle, dx, dy) == NudgeResult::Moved
            {
                self.replace_chart_config(config)?;
            }
            return Ok(());
        }
        if self.dragging
            && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
        {
            let at = self.drag_pointer;
            self.drag_pointer = [at[0] + dx, at[1] + dy];
            if drag.drag_from(&mut config, at, dx, dy) == NudgeResult::Moved {
                self.replace_chart_config(config)?;
            }
        }
        Ok(())
    }

    pub fn on_release(&mut self) {
        self.dragging = false;
        self.resizing = None;
        self.annotation_pointer = false;
    }

    /// `true` while a chart element or an annotation is selected.
    pub fn has_selection(&self) -> bool {
        self.chart_selection().is_some() || self.selected_annotation.is_some()
    }

    /// Compatibility entry point. Nonresident picking is disabled because
    /// exact source replay can take as long as the original render.
    pub async fn begin_stream_pick_point(
        &mut self,
        _x: f32,
        _y: f32,
        _max_distance_px: f32,
        _max_primitives_per_chunk: f64,
    ) -> Result<StreamingOperationHandle, JsValue> {
        Err(JsValue::from_str(
            "stream picking is disabled; no source replay is started",
        ))
    }

    /// Compatibility entry point; always rejects without requesting ranges.
    pub async fn begin_stream_pick_data(
        &mut self,
        _x: f32,
        _y: f32,
        _max_distance_px: f32,
        _max_primitives_per_chunk: f64,
    ) -> Result<StreamingOperationHandle, JsValue> {
        Err(JsValue::from_str(
            "stream picking is disabled; no source replay is started",
        ))
    }

    /// Compatibility entry point; no streamed pick operation can start.
    pub async fn finish_stream_pick_point(
        &mut self,
        _handle: &StreamingOperationHandle,
    ) -> Result<JsValue, JsValue> {
        Err(JsValue::from_str("stream picking is disabled"))
    }

    /// Compatibility entry point; no streamed pick operation can start.
    pub async fn finish_stream_pick_data(
        &mut self,
        _handle: &StreamingOperationHandle,
    ) -> Result<JsValue, JsValue> {
        Err(JsValue::from_str("stream picking is disabled"))
    }
}
