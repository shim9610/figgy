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
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        // A radar vertex maps no value to its marker.
        | DataRenderType::Radar { .. }
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
                // A radar vertex also names its category: the angle axis'
                // category at its index.
                if series.render_type.radar().is_some() {
                    let config = self.chart_config();
                    let panel = match config.layer_of_series(&series.series_id) {
                        renderer::LayerRef::Subplot(index)
                        | renderer::LayerRef::SubplotOverlay(index, _) => config
                            .subplots
                            .get(index)
                            .map_or(config, |subplot| &subplot.config),
                        renderer::LayerRef::Base
                        | renderer::LayerRef::Overlay(_)
                        | renderer::LayerRef::Annotations => config,
                    };
                    let category = panel
                        .bottom_x
                        .categories
                        .as_deref()
                        .and_then(|categories| categories.get(*point_index))
                        .ok_or_else(|| js_err("selected category no longer exists"))?;
                    result["category_id"] = serde_json::json!(category.id);
                    result["category_label"] = serde_json::json!(category.label);
                }
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
            // A box reports its category and its statistics, read back from
            // the record's GPU columns the draw used.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.box_plot().is_some() =>
            {
                let config = self.chart_config();
                let layer = config.layer_of_series(&series.series_id);
                let groups = config.bar_groups_of(layer);
                let panel = match layer {
                    renderer::LayerRef::Subplot(index)
                    | renderer::LayerRef::SubplotOverlay(index, _) => config
                        .subplots
                        .get(index)
                        .map_or(config, |subplot| &subplot.config),
                    renderer::LayerRef::Base
                    | renderer::LayerRef::Overlay(_)
                    | renderer::LayerRef::Annotations => config,
                };
                let categories = match groups.direction {
                    renderer::CategoryBarDirection::Vertical => &panel.bottom_x.categories,
                    renderer::CategoryBarDirection::Horizontal => &panel.left_y.categories,
                };
                let category = categories
                    .as_deref()
                    .and_then(|categories| categories.get(*bin_index))
                    .ok_or_else(|| js_err("selected category no longer exists"))?;
                result["category_id"] = serde_json::json!(category.id);
                result["category_label"] = serde_json::json!(category.label);
                let (_, record_id) = series.box_plot_columns();
                let record = self
                    .renderer
                    .pool()
                    .record(record_id)
                    .ok_or_else(|| js_err("the selected box's record no longer exists"))?;
                // The record's present fields, as (name, member column).
                let mut names = [""; renderer::MAX_RECORD_FIELDS];
                let mut samples: [(String, usize); renderer::MAX_RECORD_FIELDS] =
                    Default::default();
                let mut count = 0;
                for (k, (name, _)) in renderer::BOX_PLOT_RECORD.iter().enumerate() {
                    if let Some(member) = record.slot().fields[k].as_ref() {
                        names[count] = name;
                        samples[count] = (member.clone(), *bin_index);
                        count += 1;
                    }
                }
                let values = self.read_selection_samples(&samples[..count]).await?;
                result["index"] = serde_json::json!(bin_index);
                for (name, value) in names.iter().zip(&values) {
                    // A missing optional statistic (NaN) reads as null.
                    result[*name] = serde_json::json!(value.is_finite().then_some(*value));
                }
                result["direction"] = serde_json::json!(groups.direction);
            }
            // A category bar reports its category and its value, read back
            // from the GPU column the draw used.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.category_bars().is_some() =>
            {
                let config = self.chart_config();
                let layer = config.layer_of_series(&series.series_id);
                let groups = config.bar_groups_of(layer);
                let panel = match layer {
                    renderer::LayerRef::Subplot(index)
                    | renderer::LayerRef::SubplotOverlay(index, _) => config
                        .subplots
                        .get(index)
                        .map_or(config, |subplot| &subplot.config),
                    renderer::LayerRef::Base
                    | renderer::LayerRef::Overlay(_)
                    | renderer::LayerRef::Annotations => config,
                };
                let categories = match groups.direction {
                    renderer::CategoryBarDirection::Vertical => &panel.bottom_x.categories,
                    renderer::CategoryBarDirection::Horizontal => &panel.left_y.categories,
                };
                let category = categories
                    .as_deref()
                    .and_then(|categories| categories.get(*bin_index))
                    .ok_or_else(|| js_err("selected category no longer exists"))?;
                result["category_id"] = serde_json::json!(category.id);
                result["category_label"] = serde_json::json!(category.label);
                let (_, values) = series.category_bar_columns(groups.direction);
                let values = self
                    .read_selection_samples(&[(values.to_owned(), *bin_index)])
                    .await?;
                result["index"] = serde_json::json!(bin_index);
                result["value"] = serde_json::json!(values[0]);
                result["direction"] = serde_json::json!(groups.direction);
                result["mode"] = serde_json::json!(groups.mode);
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
            // A value label reports its category, its drawn string and its
            // drag offset: configuration, nothing is read back.
            PickedDataRef::ValueLabel { category_index, .. } => {
                let config = self.chart_config();
                let layer = config.layer_of_series(&series.series_id);
                let panel = match layer {
                    renderer::LayerRef::Subplot(index)
                    | renderer::LayerRef::SubplotOverlay(index, _) => config
                        .subplots
                        .get(index)
                        .map_or(config, |subplot| &subplot.config),
                    renderer::LayerRef::Base
                    | renderer::LayerRef::Overlay(_)
                    | renderer::LayerRef::Annotations => config,
                };
                let categories = if series.render_type.radar().is_some() {
                    &panel.bottom_x.categories
                } else {
                    match config.bar_groups_of(layer).direction {
                        renderer::CategoryBarDirection::Vertical => &panel.bottom_x.categories,
                        renderer::CategoryBarDirection::Horizontal => &panel.left_y.categories,
                    }
                };
                let category = categories
                    .as_deref()
                    .and_then(|categories| categories.get(*category_index))
                    .ok_or_else(|| js_err("selected category no longer exists"))?;
                let text = series
                    .value_label_text()
                    .and_then(|text| text.get(*category_index))
                    .ok_or_else(|| js_err("selected label no longer exists"))?;
                result["index"] = serde_json::json!(category_index);
                result["category_id"] = serde_json::json!(category.id);
                result["category_label"] = serde_json::json!(category.label);
                result["text"] = serde_json::json!(text);
                result["offset"] = serde_json::json!(series.value_label_offset(*category_index));
                result["label_id"] = serde_json::json!(renderer::data_config::value_label_id(
                    &series.series_id,
                    *category_index
                ));
            }
            _ => return Ok("null".into()),
        }
        serde_json::to_string(&result).map_err(js_err)
    }

    /// Move the resident typed data selection using source indices:
    /// `Config::step_selection` over the chart's series and the registered
    /// column lengths, then the moved config replaces the chart's.
    pub fn step_selection(&mut self, dx: i32, dy: i32) -> Result<bool, JsValue> {
        let mut cfg = self.chart_config().clone();
        if !cfg
            .step_selection(self.chart_series(), dx, dy, |column| {
                self.columns.get(column).copied()
            })
            .map_err(js_err)?
        {
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
    /// mapping; error-bar stems and caps name their point; line strokes and
    /// the inside of a line's fill snap to the nearest endpoint data point on
    /// the hit segment.
    /// Resolves to a JSON string
    /// `{ source_id, series_id, point_index, distance_px }`, or
    /// `undefined` when no visible primitive is within `max_distance_px`.
    /// On a chart that draws value labels a label under the pointer wins
    /// (it is drawn over the data): the hit then names the label's series,
    /// its category as `point_index`, and its `label_id`
    /// (`"series:<series_id>:label:<index>"`), which `press_value_label`
    /// takes to drag it.
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
        // A drawn value label is a typed pick target; the point pick does
        // not see it.
        let labelled = self.chart_series().iter().any(|series| {
            series
                .value_label_text()
                .is_some_and(|text| text.iter().any(|t| !t.is_empty()))
        });
        if labelled {
            let ticket = self
                .renderer
                .pick_chart_data_at(self.chart_id, [x, y], max_distance_px)
                .map_err(js_err)?;
            if let Some(renderer::PickedData::ValueLabel {
                source_id,
                series_id,
                category_index,
                distance_px,
            }) = ticket.resolve().await.map_err(js_err)?
            {
                let json = crate::picked_label_json_string(
                    source_id.as_deref(),
                    &series_id,
                    category_index,
                    distance_px,
                )
                .map_err(js_err)?;
                return Ok(JsValue::from_str(&json));
            }
        }
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

    /// Press on a value label: the label a `pick_point` or `pick_data` hit
    /// names under the pointer, by its `label_id`
    /// (`"series:<series_id>:label:<index>"`). The label becomes the typed
    /// data selection (`picked_data`, outlined by its frame), any element
    /// or annotation selection is cleared, and the drag starts: `on_move`
    /// moves the label (writing the series' `labels.offsets`) and
    /// `on_release` ends it. Returns `false` and changes nothing for an id
    /// that names no value label of the chart's series.
    /// JS: `const hit = JSON.parse(await chart.pick_point(x, y, 8));`
    /// `if (hit?.label_id) chart.press_value_label(hit.label_id);`
    pub fn press_value_label(&mut self, label_id: &str) -> Result<bool, JsValue> {
        let Some((series_id, index)) = renderer::data_config::parse_value_label_id(label_id) else {
            return Ok(false);
        };
        let Some(series) = self
            .chart_series()
            .iter()
            .find(|series| series.series_id == series_id)
        else {
            return Ok(false);
        };
        if !series
            .value_label_text()
            .is_some_and(|text| index < text.len())
        {
            return Ok(false);
        }
        let reference = renderer::config::PickedDataRef::ValueLabel {
            source_id: series.source_id.clone(),
            series_id: series.series_id.clone(),
            category_index: index,
        };
        if self.chart_selection().is_some() {
            self.renderer
                .set_chart_selection(self.chart_id, None)
                .map_err(js_err)?;
        }
        self.set_selected_annotation(None);
        let mut config = self.chart_config().clone();
        let mut picked = config.picked_data.take().unwrap_or_default();
        picked.refs = vec![reference];
        config.picked_data = Some(picked);
        self.replace_chart_config(config)?;
        self.annotation_pointer = false;
        self.resizing = None;
        self.dragging = true;
        self.value_label_pointer = true;
        Ok(true)
    }

    /// `on_move` of a drag `press_value_label` started: the selected label
    /// moves by the delta in document pixels.
    fn move_value_label(&mut self, dx: f32, dy: f32) -> Result<(), JsValue> {
        let Some(renderer::config::PickedDataRef::ValueLabel {
            series_id,
            category_index,
            ..
        }) = self
            .chart_config()
            .picked_data
            .as_ref()
            .and_then(|picked| picked.refs.first())
            .cloned()
        else {
            return Ok(());
        };
        let (dx, dy) = self.display_delta_to_document(dx, dy);
        let mut series = self.chart_series().to_vec();
        let Some(item) = series.iter_mut().find(|s| s.series_id == series_id) else {
            return Ok(());
        };
        let count = item.value_label_text().map_or(0, <[String]>::len);
        if item.drag_value_label(category_index, count, dx, dy) == NudgeResult::Moved {
            let config = self.chart_config().clone();
            self.replace_chart_state(config, series)?;
        }
        Ok(())
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
        self.value_label_pointer = false;
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
        if self.value_label_pointer {
            if self.dragging && dx.is_finite() && dy.is_finite() {
                self.move_value_label(dx, dy)?;
            }
            return Ok(());
        }
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
        // Containment is measured on the drawn parts, with the font the
        // document draws with.
        let measure = CpuTextMeasure::for_style(&config.draw_style);
        if let Some(handle) = self.resizing {
            if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                && rz.resize_by(&mut config, &measure, handle, dx, dy) == NudgeResult::Moved
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
            if drag.drag_from(&mut config, &measure, at, dx, dy) == NudgeResult::Moved {
                self.replace_chart_config(config)?;
            }
        }
        Ok(())
    }

    pub fn on_release(&mut self) {
        self.dragging = false;
        self.resizing = None;
        self.annotation_pointer = false;
        self.value_label_pointer = false;
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
