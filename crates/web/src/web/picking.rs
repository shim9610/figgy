//! Hover, picking, selection and pointer interaction.

use super::*;

/// The point style of a series that draws markers, whichever variant
/// carries it.
pub(super) fn scatter_style(rt: &DataRenderType) -> Option<&renderer::DataScatterStyleConfig> {
    match rt {
        DataRenderType::Xy(renderer::data_config::XySeries { scatter: Some(scatter), .. }) => Some(scatter),
        // A ternary series' markers, when it draws them.
        DataRenderType::Ternary { scatter, .. } => scatter.as_ref(),
        DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, .. })
        | DataRenderType::Histogram { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::RangeBars { .. }
        // A radar vertex maps no value to its marker.
        | DataRenderType::Radar { .. }
        // A streamline has no markers; its colour column colours the line.
        | DataRenderType::Streamlines { .. }
        // Nor has a parallel series; its colour column colours the rows.
        | DataRenderType::Parallel { .. }
        // An energy level is a stroke, never a marker (#203).
        | DataRenderType::EnergyLevels(_)
        // A mesh is a field over triangles (#210): no markers.
        | DataRenderType::TriMesh(_)
        | DataRenderType::Feynman(_)
        // A monotonic series draws a line, no markers (#271).
        | DataRenderType::MonotonicLine(_)
        | DataRenderType::Field(_)
        | DataRenderType::RgbImage { .. }
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
        let stored = match self.monotonic {
            Some(id) => {
                self.renderer
                    .monotonic_chart(id)
                    .and_then(|mut chart| {
                        chart.edit_config(|editor| editor.set_active_layer(layer))
                    })
                    .map_err(js_err)?;
                self.chart_config()?.active_layer()
            }
            None => self
                .renderer
                .set_active_layer(self.chart_id, layer)
                .map_err(js_err)?,
        };
        self.sync_hitmap()?;
        serde_json::to_string(&stored).map_err(js_err)
    }

    /// The layer that receives pointer interaction, as JSON, with the
    /// out-of-range fallback applied.
    /// JS: `JSON.parse(chart.active_layer())`
    pub fn active_layer(&self) -> Result<String, JsValue> {
        let layer = match self.monotonic {
            Some(_) => self.chart_config()?.active_layer(),
            None => self.renderer.active_layer(self.chart_id).map_err(js_err)?,
        };
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
    pub fn hit_test(&self, x: f32, y: f32) -> Result<Option<String>, JsValue> {
        let (display_config, _, _) = self.display_config()?;
        Ok(
            match self
                .view
                .hit_test_active_layer(&display_config, &self.hitmap, [x, y])
            {
                None => None,
                Some(LayerHit::Annotation { index, .. }) => {
                    Some(renderer::DecorationId::Annotation(index).element_id())
                }
                Some(LayerHit::Element(id)) => self.hitmap.get(id).map(|el| el.element_id()),
            },
        )
    }

    // Copy only the selected scalar pairs out of the resident GPU pool.
    // Registered source arrays are not retained or reconstructed on CPU.
    pub(super) async fn read_selection_samples(
        &self,
        samples: &[(String, usize)],
    ) -> Result<Vec<f64>, JsValue> {
        self.read_selection_lanes(samples, |[h0, h1, h2, h3, l0, l1, l2, l3]| {
            f32::from_le_bytes([h0, h1, h2, h3]) as f64
                + f32::from_le_bytes([l0, l1, l2, l3]) as f64
        })
        .await
    }

    // The selected entries of a triangle index column (#210): each index is
    // the first `u32` lane of its pair (SHADER_COMMON.md §5.14).
    pub(super) async fn read_selection_indices(
        &self,
        samples: &[(String, usize)],
    ) -> Result<Vec<u32>, JsValue> {
        self.read_selection_lanes(samples, |[i0, i1, i2, i3, ..]| {
            u32::from_le_bytes([i0, i1, i2, i3])
        })
        .await
    }

    // Copy the selected 8-byte pairs out of the resident GPU pool and decode
    // each with `decode`.
    async fn read_selection_lanes<T>(
        &self,
        samples: &[(String, usize)],
        decode: fn([u8; 8]) -> T,
    ) -> Result<Vec<T>, JsValue> {
        // Nothing to read (a violin without boxes names no record field): a
        // zero-sized buffer cannot be mapped, so no GPU work is submitted.
        if samples.is_empty() {
            return Ok(Vec::new());
        }
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
            if sender.send(result).is_err() {
                // Receiver dropped: the waiting call was cancelled.
            }
        });
        receiver.await.map_err(js_err)?.map_err(js_err)?;
        let mapped = slice.get_mapped_range().map_err(js_err)?;
        let (pairs, _) = mapped.as_chunks::<8>();
        let values = pairs.iter().map(|&pair| decode(pair)).collect();
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
            .chart_config()?
            .picked_data
            .as_ref()
            .and_then(|selection| selection.refs.first())
            .cloned()
        else {
            return Ok("null".into());
        };
        let series = self
            .chart_series()?
            .iter()
            .find(|series| series.series_id == reference.series_id())
            .cloned()
            .ok_or_else(|| js_err("selected series no longer exists"))?;
        let mut result = serde_json::json!({"identity": reference, "series_id": series.series_id});
        match &reference {
            // A monotonic series' point is a kept point of its envelope
            // (#271): its x and y are read from the envelope, not the GPU.
            PickedDataRef::Point { point_index, .. } if series.monotonic().is_some() => {
                let point = self
                    .renderer
                    .monotonic_kept_point(self.monotonic_id()?, &series.series_id, *point_index)
                    .map_err(js_err)?;
                result["index"] = serde_json::json!(point_index);
                result["x"] = serde_json::json!(point.map(|(x, _)| x));
                result["y"] = serde_json::json!(point.map(|(_, y)| y).filter(|y| y.is_finite()));
            }
            // A box plot's raw point reports its category (its position
            // column's value there, and the category it names) and its
            // value, read back from the GPU columns the draw used.
            PickedDataRef::Point { point_index, .. }
                if series
                    .render_type
                    .box_plot()
                    .is_some_and(|b| b.points.is_some()) =>
            {
                let Some(points) = series.render_type.box_plot().and_then(|b| b.points) else {
                    return Err(js_err("internal: the guard matched a box plot with points"));
                };
                let samples = [
                    (points.positions.clone(), *point_index),
                    (points.values.clone(), *point_index),
                ];
                let values = self.read_selection_samples(&samples).await?;
                result["index"] = serde_json::json!(point_index);
                result["position"] = serde_json::json!(values[0].is_finite().then_some(values[0]));
                result["value"] = serde_json::json!(values[1].is_finite().then_some(values[1]));
                let config = self.chart_config()?;
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
                let category = (values[0].is_finite() && values[0] >= 0.0)
                    .then(|| values[0].round() as usize)
                    .and_then(|k| categories.as_deref().and_then(|c| c.get(k)));
                if let Some(category) = category {
                    result["category_id"] = serde_json::json!(category.id);
                    result["category_label"] = serde_json::json!(category.label);
                }
                result["direction"] = serde_json::json!(groups.direction);
            }
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
                let three_d = self.chart_config()?.drawn_z_column(&series).is_some();
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
                // A point vector reports its components (its z component
                // where it is drawn in 3D, #211) and, when colour mapped,
                // the value its colour came from.
                let vector = match vector_columns(&series.render_type) {
                    Some(VectorColumns::Points { u, v, w, color }) => {
                        samples.push((u.to_owned(), *point_index));
                        samples.push((v.to_owned(), *point_index));
                        let w = w.filter(|_| three_d);
                        if let Some(w) = w {
                            samples.push((w.to_owned(), *point_index));
                        }
                        if let Some(color) = color {
                            samples.push((color.to_owned(), *point_index));
                        }
                        Some((w.is_some(), color.is_some()))
                    }
                    _ => None,
                };
                // A ternary point reports its third component, and a point
                // with a height column (a 3D chart's) its z, read last.
                let ternary_c = series.render_type.ternary_c_column().is_some();
                if let Some(c) = series.render_type.ternary_c_column() {
                    samples.push((c.to_owned(), *point_index));
                }
                let height = series.z_column.clone();
                if let Some(z) = &height {
                    samples.push((z.clone(), *point_index));
                }
                let values = self.read_selection_samples(&samples).await?;
                let mut last = values.len();
                if height.is_some() {
                    last -= 1;
                    result["z"] = serde_json::json!(values[last]);
                }
                if ternary_c {
                    result["c"] = serde_json::json!(values[last - 1]);
                }
                result["index"] = serde_json::json!(point_index);
                result["x"] = serde_json::json!(values[0]);
                result["y"] = serde_json::json!(values[1]);
                // A radar vertex also names its category: the angle axis'
                // category at its index.
                if series.render_type.radar().is_some() {
                    let config = self.chart_config()?;
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
                if let Some((has_w, colour_mapped)) = vector {
                    result["u"] = serde_json::json!(values[next]);
                    result["v"] = serde_json::json!(values[next + 1]);
                    next += 2;
                    if has_w {
                        result["w"] = serde_json::json!(values[next]);
                        next += 1;
                    }
                    if colour_mapped {
                        result["color_value"] = serde_json::json!(values[next]);
                    }
                }
                // A parallel row reports every axis's value (`values`, in
                // axis order) and, mapped, its colour value, read from the
                // GPU columns the draw used (#141).
                if let Some(parallel) = series.render_type.parallel() {
                    let mut samples: Vec<(String, usize)> = parallel
                        .columns
                        .iter()
                        .map(|column| (column.clone(), *point_index))
                        .collect();
                    if let Some(column) = parallel.color_column {
                        samples.push((column.to_owned(), *point_index));
                    }
                    let read = self.read_selection_samples(&samples).await?;
                    let axes = parallel.columns.len();
                    result["values"] = serde_json::json!(read[..axes]);
                    if parallel.color_column.is_some() {
                        result["color_value"] = serde_json::json!(read[axes]);
                    }
                }
            }
            // A box reports its category and its statistics, read back from
            // the record's GPU columns the draw used.
            PickedDataRef::HistogramBin {
                bin_index, half, ..
            } if series.render_type.box_plot().is_some() => {
                // A split violin's picked half, as the selection names it.
                if let Some(half) = half {
                    result["half"] = serde_json::json!(half);
                }
                let config = self.chart_config()?;
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
                // The record's present fields, as (name, member column); a
                // violin without boxes has none.
                let mut names = [""; renderer::MAX_RECORD_FIELDS];
                let mut samples: [(String, usize); renderer::MAX_RECORD_FIELDS] =
                    Default::default();
                let mut count = 0;
                if !record_id.is_empty() {
                    let record = self
                        .renderer
                        .pool()
                        .record(record_id)
                        .ok_or_else(|| js_err("the selected box's record no longer exists"))?;
                    for (k, (name, _)) in renderer::BOX_PLOT_RECORD.iter().enumerate() {
                        if let Some(member) = record.slot().fields[k].as_ref() {
                            names[count] = name;
                            samples[count] = (member.clone(), *bin_index);
                            count += 1;
                        }
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
            // A range bar reports the values its columns hold for its row,
            // read back from the GPU columns the draw used: its position
            // (and, on a categorical axis, the category there), end, start
            // and, when the series has them, its low and high, total flag
            // or state, and progress. Never a difference or a duration:
            // those are the host's to derive.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.range_bars().is_some() =>
            {
                use renderer::data_config::RangeColor;
                let Some(bars) = series.render_type.range_bars() else {
                    return Err(js_err("internal: the guard matched a range bar series"));
                };
                let colour = match bars.color_by {
                    RangeColor::Single => None,
                    RangeColor::Direction { total, .. } => {
                        total.as_ref().map(|t| ("total", t.column.as_str()))
                    }
                    RangeColor::State { column, .. } => Some(("state", column.as_str())),
                };
                let roles = [
                    Some(("position", series.x_column.as_str())),
                    Some(("end", series.y_column.as_str())),
                    Some(("start", bars.start)),
                    bars.whiskers.map(|w| ("low", w.low.as_str())),
                    bars.whiskers.map(|w| ("high", w.high.as_str())),
                    colour,
                    bars.progress.map(|p| ("progress", p.column.as_str())),
                ];
                let mut names = [""; 7];
                let mut samples: [(String, usize); 7] = Default::default();
                let mut count = 0;
                for (name, column) in roles.into_iter().flatten() {
                    names[count] = name;
                    samples[count] = (column.to_owned(), *bin_index);
                    count += 1;
                }
                let values = self.read_selection_samples(&samples[..count]).await?;
                result["index"] = serde_json::json!(bin_index);
                for (name, value) in names.iter().zip(&values) {
                    result[*name] = serde_json::json!(value.is_finite().then_some(*value));
                }
                let config = self.chart_config()?;
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
                let at = values
                    .first()
                    .copied()
                    .filter(|v| v.is_finite() && *v >= 0.0);
                if let Some(category) = categories
                    .as_deref()
                    .zip(at)
                    .and_then(|(categories, at)| categories.get(at.round() as usize))
                {
                    result["category_id"] = serde_json::json!(category.id);
                    result["category_label"] = serde_json::json!(category.label);
                }
                result["direction"] = serde_json::json!(groups.direction);
            }
            // A category bar reports its category and its value, read back
            // from the GPU column the draw used.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.category_bars().is_some() =>
            {
                let config = self.chart_config()?;
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
                let image = series.render_type.rgb_image();
                let (matrix, vector) = match &series.render_type {
                    DataRenderType::Field(renderer::data_config::FieldSeries {
                        matrix, ..
                    }) => (matrix, None),
                    // An image's lattice is its red grid's; the green and
                    // blue values are read beside it (#139).
                    DataRenderType::RgbImage { red, .. } => (red, None),
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
                // A hexagonal grid's odd columns are raised by half the row
                // pitch (#134): the centre reported is the raised one.
                let hexagonal = matches!(matrix.grid_layout, GridLayout::Hexagonal);
                if hexagonal {
                    samples.extend([(series.y_column.clone(), 0), (series.y_column.clone(), 1)]);
                }
                if let Some((v, color)) = vector {
                    samples.push((grid_column(v)?, sample_index));
                    if let Some(color) = color {
                        samples.push((grid_column(color)?, sample_index));
                    }
                }
                if let Some([_, green, blue]) = image {
                    samples.push((grid_column(green)?, sample_index));
                    samples.push((grid_column(blue)?, sample_index));
                }
                // A surface coloured by a second grid also reports the value
                // its colour came from.
                let surface_color = series
                    .render_type
                    .surface_style()
                    .and_then(|style| style.color_matrix.as_ref())
                    .filter(|_| vector.is_none());
                if let Some(color) = surface_color {
                    samples.push((grid_column(color)?, sample_index));
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
                } else if hexagonal {
                    values[1] + (x_index % 2) as f64 * (values[4] - values[3]) * 0.5
                } else {
                    values[1]
                });
                let mut next = if edges || hexagonal { 5 } else { 3 };
                match vector {
                    None if image.is_some() => {
                        result["red"] = serde_json::json!(values[2]);
                        result["green"] = serde_json::json!(values[next]);
                        result["blue"] = serde_json::json!(values[next + 1]);
                    }
                    None => {
                        result["value"] = serde_json::json!(values[2]);
                        if surface_color.is_some() {
                            result["color_value"] = serde_json::json!(values[next]);
                        }
                    }
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
                let config = self.chart_config()?;
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
            // A Feynman part (#234) reports its row from the SSOT; nothing
            // is read back. A vertex reports where it is drawn: its `at`, or
            // the layout's place while it has none (`placed: false`).
            PickedDataRef::FeynmanVertex { vertex_index, .. } => {
                let diagram = series
                    .feynman()
                    .ok_or_else(|| js_err("selected series is not a Feynman diagram"))?;
                let vertex = diagram
                    .vertices
                    .get(*vertex_index)
                    .ok_or_else(|| js_err("selected vertex no longer exists"))?;
                // The wrapper commits every layout on registration, so this
                // reads `at` without allocating; a host-built document
                // without it lays the diagram out once.
                let position = match vertex.at {
                    Some(at) => at,
                    None => diagram.positions().map_err(js_err)?[*vertex_index],
                };
                result["index"] = serde_json::json!(vertex_index);
                result["id"] = serde_json::json!(vertex.id);
                result["role"] = serde_json::to_value(vertex.role).map_err(js_err)?;
                result["position"] = serde_json::json!(position);
                result["placed"] = serde_json::json!(vertex.at.is_some());
            }
            PickedDataRef::FeynmanPropagator {
                propagator_index, ..
            } => {
                let diagram = series
                    .feynman()
                    .ok_or_else(|| js_err("selected series is not a Feynman diagram"))?;
                let propagator = diagram
                    .propagators
                    .get(*propagator_index)
                    .ok_or_else(|| js_err("selected propagator no longer exists"))?;
                result["index"] = serde_json::json!(propagator_index);
                result["id"] = serde_json::json!(propagator.id);
                result["from"] = serde_json::json!(propagator.from);
                result["to"] = serde_json::json!(propagator.to);
                result["particle"] = serde_json::to_value(propagator.particle).map_err(js_err)?;
                result["bend"] = serde_json::json!(propagator.bend);
            }
            PickedDataRef::FeynmanLabel { part, index, .. } => {
                let diagram = series
                    .feynman()
                    .ok_or_else(|| js_err("selected series is not a Feynman diagram"))?;
                let offset = diagram
                    .label_offset(*part, *index)
                    .ok_or_else(|| js_err("selected label no longer exists"))?;
                result["index"] = serde_json::json!(index);
                result["part"] = serde_json::to_value(part).map_err(js_err)?;
                result["offset"] = serde_json::json!(offset);
            }
            // A mesh triangle reports its three vertex indices, read from
            // its index column's `u32` lanes, then their coordinates and its
            // values (one per vertex, or its face value), read back from
            // the GPU columns the draw used (#210).
            PickedDataRef::MeshTriangle { triangle_index, .. } => {
                let mesh = series
                    .tri_mesh()
                    .ok_or_else(|| js_err("selected series is not a triangle mesh"))?;
                let member = self
                    .renderer
                    .pool()
                    .triangles(&mesh.triangles)
                    .map(|triangles| triangles.slot().indices.clone())
                    .ok_or_else(|| js_err("selected triangles no longer exist"))?;
                let first = triangle_index
                    .checked_mul(3)
                    .ok_or_else(|| js_err("selected triangle is outside the mesh"))?;
                let corners = self
                    .read_selection_indices(&[
                        (member.clone(), first),
                        (member.clone(), first + 1),
                        (member, first + 2),
                    ])
                    .await?;
                let faces = mesh.values.at == renderer::tri_mesh::MeshValueLocation::Faces;
                // On a 3D chart the triangle's heights too (#210), read last.
                let height = self
                    .chart_config()?
                    .drawn_z_column(&series)
                    .map(str::to_owned);
                let mut samples: [(String, usize); 12] = Default::default();
                for (corner, &vertex) in corners.iter().enumerate() {
                    let vertex = vertex as usize;
                    samples[corner] = (series.x_column.clone(), vertex);
                    samples[3 + corner] = (series.y_column.clone(), vertex);
                    samples[6 + corner] = (mesh.values.column.clone(), vertex);
                }
                let mut count = if faces {
                    samples[6] = (mesh.values.column.clone(), *triangle_index);
                    7
                } else {
                    9
                };
                let heights_at = height.map(|z| {
                    for (corner, &vertex) in corners.iter().enumerate() {
                        samples[count + corner] = (z.clone(), vertex as usize);
                    }
                    count += 3;
                    count - 3
                });
                let values = self.read_selection_samples(&samples[..count]).await?;
                let finite = |v: f64| v.is_finite().then_some(v);
                result["index"] = serde_json::json!(triangle_index);
                result["vertices"] = serde_json::json!(corners);
                let triple = |i: usize| {
                    serde_json::json!([
                        finite(values[i]),
                        finite(values[i + 1]),
                        finite(values[i + 2])
                    ])
                };
                result["x"] = triple(0);
                result["y"] = triple(3);
                if faces {
                    result["value"] = serde_json::json!(finite(values[6]));
                } else {
                    result["values"] = triple(6);
                }
                if let Some(at) = heights_at {
                    result["z"] = triple(at);
                }
                result["at"] = serde_json::json!(mesh.values.at);
            }
            _ => return Ok("null".into()),
        }
        serde_json::to_string(&result).map_err(js_err)
    }

    /// Move the resident typed data selection using source indices
    /// (`Renderer::step_chart_selection`): `Config::step_selection` over
    /// the chart's series and the registered column lengths, and a
    /// monotonic series' point over the points its envelope keeps (#271).
    pub fn step_selection(&mut self, dx: i32, dy: i32) -> Result<bool, JsValue> {
        let stepped = match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .and_then(|mut chart| chart.step_selection(dx, dy)),
            None => self.renderer.step_chart_selection(self.chart_id, dx, dy),
        };
        if !stepped.map_err(js_err)? {
            return Ok(false);
        }
        self.sync_hitmap()?;
        Ok(true)
    }

    /// Zoom a 3D chart's camera (`Config.view_3d`): the eye distance is
    /// divided by `factor` and clamped (`Config::zoom_view`). Returns whether
    /// the view changed; a flat chart never does.
    pub fn zoom_view(&mut self, factor: f64) -> Result<bool, JsValue> {
        let mut cfg = self.chart_config()?.clone();
        if !cfg.zoom_view(factor) {
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
        match self.monotonic {
            Some(id) => self
                .renderer
                .monotonic_chart(id)
                .map_err(js_err)?
                .prepare_picking()
                .map_err(js_err),
            None => self
                .renderer
                .prepare_gpu_picking_for_chart(self.chart_id)
                .map_err(js_err),
        }
    }

    /// Whether the shown chart draws from a stream: a monotonic chart
    /// never does (#271).
    fn shows_stream(&self) -> bool {
        self.monotonic.is_none() && self.renderer.is_streaming_chart(self.chart_id)
    }

    /// The request of a pick at canvas pixel `at` on a monotonic chart, as
    /// the window builds it for a general one: the document fitted to the
    /// surface.
    fn monotonic_pick_request(
        &self,
        id: renderer::MonotonicChartId,
        at: [f32; 2],
        max_distance_px: f32,
    ) -> Result<renderer::GpuPickRequest, JsValue> {
        let document = self
            .renderer
            .monotonic_config(id)
            .map_err(js_err)?
            .document_area();
        let (display_scale, display_panel_px) =
            renderer::fit_display_panel((document.width, document.height), self.surface_size);
        Ok(renderer::GpuPickRequest {
            canvas_position_px: at,
            display_panel_px,
            display_scale,
            max_distance_px,
        })
    }

    /// A typed data pick on the shown chart.
    fn data_pick_ticket(
        &mut self,
        at: [f32; 2],
        max_distance_px: f32,
    ) -> Result<renderer::GpuDataPickTicket, JsValue> {
        match self.monotonic {
            Some(id) => {
                let request = self.monotonic_pick_request(id, at, max_distance_px)?;
                self.renderer
                    .monotonic_chart(id)
                    .and_then(|mut chart| chart.pick_data(request))
                    .map_err(js_err)
            }
            None => self
                .renderer
                .pick_chart_data_at(self.chart_id, at, max_distance_px)
                .map_err(js_err),
        }
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
        if self.shows_stream() {
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
        // An energy-level series (#203) is picked on the CPU through the
        // same typed query: its levels, arrows and labels.
        // So is a Feynman diagram (#234): its vertices, lines and labels.
        let labelled = self.chart_series()?.iter().any(|series| {
            series.is_dataless()
                || series
                    .value_label_text()
                    .is_some_and(|text| text.iter().any(|t| !t.is_empty()))
        });
        if labelled {
            let ticket = self.data_pick_ticket([x, y], max_distance_px)?;
            match ticket.resolve().await.map_err(js_err)? {
                Some(renderer::PickedData::ValueLabel {
                    source_id,
                    series_id,
                    category_index,
                    part: None,
                    distance_px,
                }) => {
                    let json = crate::picked_label_json_string(
                        source_id.as_deref(),
                        &series_id,
                        category_index,
                        distance_px,
                    )
                    .map_err(js_err)?;
                    return Ok(JsValue::from_str(&json));
                }
                // An energy-level hit is answered whole: its kind, row and
                // (for a label) part, for `press_energy_label`.
                Some(picked) if self.is_dataless_series(picked.series_id())? => {
                    let json = crate::picked_data_json_string(&picked).map_err(js_err)?;
                    return Ok(JsValue::from_str(&json));
                }
                _ => {}
            }
        }
        let ticket = match self.monotonic {
            Some(id) => {
                let request = self.monotonic_pick_request(id, [x, y], max_distance_px)?;
                self.renderer
                    .monotonic_chart(id)
                    .map_err(js_err)?
                    .pick(request)
                    .map_err(js_err)?
            }
            None => self
                .renderer
                .pick_chart_at(self.chart_id, [x, y], max_distance_px)
                .map_err(js_err)?,
        };
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
            .chart_series()?
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
            part: None,
        };
        if self.chart_selection()?.is_some() {
            self.set_chart_selection(None)?;
        }
        self.set_selected_annotation(None);
        let mut config = self.chart_config()?.clone();
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

    /// Whether `series_id` names a DataLess series of the chart: an
    /// energy-level series or a Feynman diagram.
    fn is_dataless_series(&self, series_id: &str) -> Result<bool, JsValue> {
        Ok(self
            .chart_series()?
            .iter()
            .any(|s| s.series_id == series_id && s.is_dataless()))
    }

    /// Press on a part of a Feynman series (#234), as a `pick_point` hit
    /// names it: `kind` is `"feynman_vertex"`, `"feynman_propagator"` or
    /// `"feynman_label"` (with `part` `"vertex"`, `"propagator"` or
    /// `"momentum"`), `index` its row. The part becomes the typed selection
    /// and the drag starts: `on_move` moves a vertex (the first drag commits
    /// the layout) or bends a propagator through `Renderer::drag_feynman`,
    /// and moves a label through the value-label drag, as
    /// `press_value_label` does. Returns `false` and changes nothing for a
    /// row that does not exist.
    /// JS: `if (hit?.kind?.startsWith("feynman_")) chart.press_feynman(hit.series_id, hit.kind, hit.index, hit.part);`
    pub fn press_feynman(
        &mut self,
        series_id: &str,
        kind: &str,
        index: usize,
        part: Option<String>,
    ) -> Result<bool, JsValue> {
        use renderer::config::PickedDataRef;
        use renderer::{FeynmanHit, FeynmanLabelPart};
        let Some(series) = self
            .chart_series()?
            .iter()
            .find(|series| series.series_id == series_id)
        else {
            return Ok(false);
        };
        let Some(diagram) = series.feynman() else {
            return Ok(false);
        };
        let (source_id, series_id) = (series.source_id.clone(), series.series_id.clone());
        let (drag, reference) = match kind {
            "feynman_vertex" if index < diagram.vertices.len() => (
                FeynmanHit::Vertex(index),
                PickedDataRef::FeynmanVertex {
                    source_id,
                    series_id: series_id.clone(),
                    vertex_index: index,
                },
            ),
            "feynman_propagator" if index < diagram.propagators.len() => (
                FeynmanHit::Propagator(index),
                PickedDataRef::FeynmanPropagator {
                    source_id,
                    series_id: series_id.clone(),
                    propagator_index: index,
                },
            ),
            "feynman_label" => {
                let part = match part.as_deref() {
                    Some("vertex") => FeynmanLabelPart::Vertex,
                    Some("propagator") => FeynmanLabelPart::Propagator,
                    Some("momentum") => FeynmanLabelPart::Momentum,
                    _ => return Ok(false),
                };
                if diagram.label_offset(part, index).is_none() {
                    return Ok(false);
                }
                (
                    FeynmanHit::Label(part, index),
                    PickedDataRef::FeynmanLabel {
                        source_id,
                        series_id: series_id.clone(),
                        part,
                        index,
                    },
                )
            }
            _ => return Ok(false),
        };
        if self.chart_selection()?.is_some() {
            self.set_chart_selection(None)?;
        }
        self.set_selected_annotation(None);
        let mut config = self.chart_config()?.clone();
        let mut picked = config.picked_data.take().unwrap_or_default();
        picked.refs = vec![reference];
        config.picked_data = Some(picked);
        self.replace_chart_config(config)?;
        self.annotation_pointer = false;
        self.resizing = None;
        self.dragging = true;
        // A label moves through the value-label drag (`move_value_label`);
        // a vertex or a line through the panel's axes.
        if let FeynmanHit::Label(..) = drag {
            self.value_label_pointer = true;
            self.feynman_pointer = None;
        } else {
            self.value_label_pointer = false;
            self.feynman_pointer = Some((series_id, drag));
        }
        Ok(true)
    }

    /// Commit the automatic layout of the Feynman series `series_id`
    /// (#234): every unplaced vertex gets the place it is drawn at.
    /// Returns whether anything was written. The wrapper commits it on
    /// registration already; a host that edits the series in place calls
    /// this to fix the new vertices.
    pub fn auto_layout(&mut self, series_id: &str) -> Result<bool, JsValue> {
        let laid_out = self
            .renderer
            .auto_layout(self.chart_id, series_id)
            .map_err(js_err)?;
        if laid_out {
            self.request_host_redraw();
        }
        Ok(laid_out)
    }

    /// Press on an energy-level label (#203): row `index` of the list `part`
    /// (`"level"`, `"transition"`, `"limit"` or `"band"`) of series `series_id`, as a
    /// `pick_point` hit names it. The label becomes the typed selection and
    /// the drag starts, as `press_value_label` does; `on_move` writes the
    /// row's `label_offset`. Returns `false` and changes nothing for a row
    /// without a label.
    /// JS: `if (hit?.kind === "value_label" && hit.part) chart.press_energy_label(hit.series_id, hit.part, hit.category_index);`
    pub fn press_energy_label(
        &mut self,
        series_id: &str,
        part: &str,
        index: usize,
    ) -> Result<bool, JsValue> {
        use renderer::energy_levels::LabelPart;
        let part = match part {
            "level" => LabelPart::Level,
            "transition" => LabelPart::Transition,
            "limit" => LabelPart::Limit,
            "band" => LabelPart::Band,
            _ => return Ok(false),
        };
        let Some(series) = self
            .chart_series()?
            .iter()
            .find(|series| series.series_id == series_id)
        else {
            return Ok(false);
        };
        let Some(levels) = series.energy_levels() else {
            return Ok(false);
        };
        if levels.label_offset(part, index).is_none() {
            return Ok(false);
        }
        let reference = renderer::config::PickedDataRef::ValueLabel {
            source_id: series.source_id.clone(),
            series_id: series.series_id.clone(),
            category_index: index,
            part: Some(part),
        };
        if self.chart_selection()?.is_some() {
            self.set_chart_selection(None)?;
        }
        self.set_selected_annotation(None);
        let mut config = self.chart_config()?.clone();
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

    /// `on_move` of a drag `press_value_label` or `press_energy_label`
    /// started: the selected label moves by the delta in document pixels.
    fn move_value_label(&mut self, dx: f32, dy: f32) -> Result<(), JsValue> {
        // A Feynman label moves its row's own offset (#234).
        if let Some(renderer::config::PickedDataRef::FeynmanLabel {
            series_id,
            part,
            index,
            ..
        }) = self
            .chart_config()?
            .picked_data
            .as_ref()
            .and_then(|picked| picked.refs.first())
            .cloned()
        {
            let (dx, dy) = self.display_delta_to_document(dx, dy)?;
            let mut series = self.chart_series()?.to_vec();
            let Some(diagram) = series
                .iter_mut()
                .find(|s| s.series_id == series_id)
                .and_then(SeriesConfig::feynman_mut)
            else {
                return Ok(());
            };
            if diagram.drag_label(part, index, dx, dy) == NudgeResult::Moved {
                let config = self.chart_config()?.clone();
                self.replace_chart_state(config, series)?;
            }
            return Ok(());
        }
        let Some(renderer::config::PickedDataRef::ValueLabel {
            series_id,
            category_index,
            part,
            ..
        }) = self
            .chart_config()?
            .picked_data
            .as_ref()
            .and_then(|picked| picked.refs.first())
            .cloned()
        else {
            return Ok(());
        };
        let (dx, dy) = self.display_delta_to_document(dx, dy)?;
        let mut series = self.chart_series()?.to_vec();
        let Some(item) = series.iter_mut().find(|s| s.series_id == series_id) else {
            return Ok(());
        };
        // An energy-level label moves its row's own offset (#203).
        if let (Some(part), Some(levels)) = (part, item.energy_levels_mut()) {
            if levels.drag_label(part, category_index, dx, dy) == NudgeResult::Moved {
                let config = self.chart_config()?.clone();
                self.replace_chart_state(config, series)?;
            }
            return Ok(());
        }
        let count = item.value_label_text().map_or(0, <[String]>::len);
        if item.drag_value_label(category_index, count, dx, dy) == NudgeResult::Moved {
            let config = self.chart_config()?.clone();
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
        if self.shows_stream() {
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
        let ticket = self.data_pick_ticket([x, y], max_distance_px)?;
        let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
            return Ok(JsValue::UNDEFINED);
        };
        let json = crate::picked_data_json_string(&picked).map_err(js_err)?;
        Ok(JsValue::from_str(&json))
    }

    /// Replace the chart's selection (`picked_data`). Passing JSON `null`
    /// clears it.
    pub fn set_picked_data(&mut self, json: &str) -> Result<(), JsValue> {
        let picked: Option<renderer::DataSelectionsConfig> =
            serde_json::from_str(json).map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
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
        self.plot_resize = None;
        // Borrow the renderer field alone, so the pointer state below stays
        // assignable while the placed config is in use.
        let (display_config, _, _) = match self.monotonic {
            Some(id) => self.renderer.monotonic_display_config(id),
            None => self.renderer.chart_display_config(self.chart_id),
        }
        .map_err(js_err)?;
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
            if self.chart_selection()?.is_some() {
                self.set_chart_selection(None)?;
            }
            self.set_selected_annotation(Some(index));
            self.resizing = None;
            self.dragging = true;
            self.annotation_pointer = true;
            return Ok(true);
        }
        self.annotation_pointer = false;
        let selected = self.chart_selection()?;
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
            self.drag_pointer = self.display_point_to_document(x, y)?;
            let (horizontal, vertical) = rz.resize_targets(handle);
            if moves_subplot_frame(horizontal.as_ref()) || moves_subplot_frame(vertical.as_ref()) {
                self.hold_display()?;
            }
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
            self.set_chart_selection(new_sel)?;
        }
        if new_sel.is_some() || routes_annotations {
            self.set_selected_annotation(None);
        }
        self.resizing = None;
        self.dragging = dragging;
        self.drag_pointer = self.display_point_to_document(x, y)?;
        let frame = new_sel
            .and_then(|id| self.hitmap.get(id))
            .and_then(|el| el.as_draggable())
            .map(|drag| drag.nudge_element());
        if moves_subplot_frame(frame.as_ref()) {
            self.hold_display()?;
        }
        Ok(new_sel.is_some())
    }

    /// Pointer move with frame delta — drags or resizes the selection.
    /// After a press on an annotation the delta moves that item (or the
    /// pressed handle) through the frame's anchor space.
    pub fn on_move(&mut self, dx: f32, dy: f32) -> Result<(), JsValue> {
        if let Some((series_id, drag)) = self.feynman_pointer.clone() {
            if self.dragging && dx.is_finite() && dy.is_finite() {
                let (dx, dy) = self.display_delta_to_document(dx, dy)?;
                let moved = self
                    .renderer
                    .drag_feynman(self.chart_id, &series_id, drag, dx, dy)
                    .map_err(js_err)?;
                if moved == NudgeResult::Moved {
                    self.request_host_redraw();
                }
            }
            return Ok(());
        }
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
        let Some(id) = self.chart_selection()? else {
            return Ok(());
        };
        let (dx, dy) = self.display_delta_to_document(dx, dy)?;
        let mut config = self.chart_config()?.clone();
        // Containment is measured on the drawn parts, with the font the
        // document draws with.
        let measure = CpuTextMeasure::for_style(&config.draw_style);
        if let Some(handle) = self.resizing {
            let at = self.drag_pointer;
            self.drag_pointer = [at[0] + dx, at[1] + dy];
            let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable()) else {
                return Ok(());
            };
            let plot_edge = |edge: &Option<renderer::layout::Element>| {
                matches!(edge, Some(renderer::layout::Element::DataAreaEdge(_)))
            };
            let (horizontal, vertical) = rz.resize_targets(handle);
            if (plot_edge(&horizontal) || plot_edge(&vertical)) && self.has_monotonic()? {
                // A plot-area resize of a monotonic chart is a resize, which
                // takes the groups' sources (#271): the gesture proposes the
                // plot rect and the host applies it on release
                // (`take_plot_resize_request`, `resize_monotonic`).
                let mut proposed = self.plot_resize.take().unwrap_or(config);
                let _ = rz.resize_from(&mut proposed, &measure, handle, at, dx, dy);
                self.plot_resize = Some(proposed);
            } else if rz.resize_from(&mut config, &measure, handle, at, dx, dy)
                == NudgeResult::Moved
            {
                self.drag_chart_config(config)?;
            }
            return Ok(());
        }
        if self.dragging
            && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
        {
            let at = self.drag_pointer;
            self.drag_pointer = [at[0] + dx, at[1] + dy];
            if drag.drag_from(&mut config, &measure, at, dx, dy) == NudgeResult::Moved {
                self.drag_chart_config(config)?;
            }
        }
        Ok(())
    }

    /// Pointer release: ends the drag or handle drag, and releases a held
    /// display placement so a document a subplot drag grew fits the canvas
    /// again (#205).
    pub fn on_release(&mut self) -> Result<(), JsValue> {
        if let Some(proposed) = self.plot_resize.take() {
            let current = self.chart_config()?.data_area().ok().map(|area| area.0);
            self.plot_resize_request = proposed
                .data_area()
                .ok()
                .map(|area| area.0)
                .filter(|&area| Some(area) != current);
        }
        self.dragging = false;
        self.resizing = None;
        self.annotation_pointer = false;
        self.value_label_pointer = false;
        self.feynman_pointer = None;
        if self.display_held {
            self.display_held = false;
            self.renderer
                .hold_chart_display(self.chart_id, false)
                .map_err(js_err)?;
        }
        Ok(())
    }

    /// Applies the config a drag moved. On a monotonic chart (#271) a move
    /// that would move its x-to-pixel mapping (a frame drag that changes the
    /// document's size, say) is refused by its document editor, and the
    /// drag keeps the chart as it is; a plot-area resize is proposed to the
    /// host instead (`take_plot_resize_request`). Every other drag applies
    /// as on any chart.
    fn drag_chart_config(&mut self, config: renderer::Config) -> Result<(), JsValue> {
        let Some(id) = self.monotonic else {
            return self.replace_chart_config(config);
        };
        match self
            .renderer
            .monotonic_chart(id)
            .and_then(|mut chart| chart.edit_config(|editor| editor.assign(config)))
            .map_err(js_err)?
        {
            Ok(()) => self.sync_hitmap(),
            // The drag would move the mapping: the chart stays as it is.
            Err(_) => Ok(()),
        }
    }

    /// Hold the display placement while a subplot frame drags (#205). A
    /// chart with monotonic series is never held (#271): the release would
    /// fit the document to the canvas again and move a series' mapping
    /// without its source.
    fn hold_display(&mut self) -> Result<(), JsValue> {
        if !self.display_held && !self.has_monotonic()? {
            self.renderer
                .hold_chart_display(self.chart_id, true)
                .map_err(js_err)?;
            self.display_held = true;
        }
        Ok(())
    }

    /// `true` while a chart element or an annotation is selected.
    pub fn has_selection(&self) -> Result<bool, JsValue> {
        Ok(self.chart_selection()?.is_some() || self.selected_annotation.is_some())
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

/// Whether a drag or handle target moves a subplot's frame (#205).
fn moves_subplot_frame(element: Option<&renderer::layout::Element>) -> bool {
    matches!(
        element,
        Some(
            renderer::layout::Element::SubplotFrame(_)
                | renderer::layout::Element::SubplotFrameEdge(..)
        )
    )
}
