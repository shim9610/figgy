//! Hover, picking, selection and pointer interaction.

use super::*;

mod selection_details;

/// The point style of a series that draws markers, whichever variant
/// carries it.
pub(super) fn scatter_style(rt: &DataRenderType) -> Option<&renderer::DataScatterStyleConfig> {
    match rt {
        DataRenderType::Xy(renderer::data_config::XySeries { scatter: Some(scatter), .. }) => Some(scatter),
        // A ternary series' markers, when it draws them.
        DataRenderType::Ternary(TernarySeries { scatter, .. }) => scatter.as_ref(),
        DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, .. })
        | DataRenderType::Histogram(_)
        | DataRenderType::CategoryBars(_)
        | DataRenderType::BoxPlot(_)
        | DataRenderType::RangeBars(_)
        | DataRenderType::RangeLinks(_)
        // A radar vertex maps no value to its marker.
        | DataRenderType::Radar(_)
        // A streamline has no markers; its colour column colours the line.
        | DataRenderType::Streamlines(_)
        // Nor has a parallel series; its colour column colours the rows.
        | DataRenderType::Parallel(_)
        // An energy level is a stroke, never a marker (#203).
        | DataRenderType::EnergyLevels(_)
        // A mesh is a field over triangles (#210): no markers.
        | DataRenderType::TriMesh(_)
        | DataRenderType::Feynman(_)
        // A monotonic series draws a line, no markers (#271).
        | DataRenderType::MonotonicLine(_)
        | DataRenderType::Field(_)
        | DataRenderType::RgbImage(_)
        | DataRenderType::Vector(_) => None,
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn active_layer(&self) -> Result<String, JsValue> {
        let layer = match self.monotonic {
            Some(_) => self.chart_config()?.active_layer(),
            None => self.renderer.active_layer(self.chart_id).map_err(js_err)?,
        };
        serde_json::to_string(&layer).map_err(js_err)
    }

    // ---- pointer interaction (coordinates in canvas pixels) ----

    /// Hit-test the chart chrome at canvas pixel `(x, y)` — returns the
    /// stable id of the element under the point (where padded hit regions
    /// overlap, the one the pointer is visually on, #272) (`"data_area"`, `"axis_bottom"`,
    /// `"tick_labels_left"`, `"axis_title_left"`, `"colorbar"`,
    /// `"colorbar_axis"`, `"colorbar_tick_labels"`,
    /// `"colorbar_title"`, …) or `null`. Pure geometry, no selection state
    /// change: the renderer's own layout answers, so hosts don't have to
    /// re-derive box positions for hover cursors / context UI. The set of
    /// registered elements follows the active layer: while an overlay is
    /// active its own Y axis answers as `"overlay_<n>:axis_right"`,
    /// `"overlay_<n>:tick_labels_right"` and `"overlay_<n>:axis_title_right"`
    /// (with `link_y` on, the parent's left axis, labels and title answer
    /// instead), its own X axis, while `link_x` is off, as
    /// `"overlay_<n>:axis_top"`, `"overlay_<n>:tick_labels_top"` and
    /// `"overlay_<n>:axis_title_top"` in place of the parent's top axis, and
    /// its legend as `"overlay_<n>:legend"`.
    /// While the annotation layer is active (and shown) its drawn items
    /// sit on top and answer as `"annotation_<n>"`; on every other layer
    /// they are not registered and clicks pass through them.
    ///
    /// `x`, `y` are physical canvas pixels (the canvas's `width`/`height`
    /// space, not CSS pixels). Returns the plain id string, not JSON.
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
    /// Compile the exact pick pipelines a pick at `at` needs through the
    /// browser's async API before the pick submits (#299), so the first
    /// pick does not compile them on the page's thread. A monotonic chart
    /// picks through its own query and needs none.
    async fn compile_pick_pipelines(
        &mut self,
        at: [f32; 2],
        max_distance_px: f32,
    ) -> Result<(), JsValue> {
        if self.monotonic.is_some() {
            return Ok(());
        }
        self.renderer
            .compile_pick_pipelines_at(self.chart_id, at, max_distance_px)
            .await
            .map_err(js_err)
    }

    /// Compile the typed data pick kernels a data pick at `at` dispatches
    /// through the browser's async API before it submits (#327); kernels
    /// other charts need wait for their own first pick. A monotonic chart
    /// draws a line, which the point query answers, and needs none.
    async fn compile_data_pick_pipelines(
        &mut self,
        at: [f32; 2],
        max_distance_px: f32,
    ) -> Result<(), JsValue> {
        if self.monotonic.is_some() {
            return Ok(());
        }
        self.renderer
            .compile_data_pick_pipelines_at(self.chart_id, at, max_distance_px)
            .await
            .map_err(js_err)
    }

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
    ///
    /// Resolves to JSON text (`FiggyJson`), not an object. `x`, `y` and
    /// `max_distance_px` are physical canvas pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson | undefined")]
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
        self.compile_pick_pipelines([x, y], max_distance_px).await?;
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
            self.compile_data_pick_pipelines([x, y], max_distance_px)
                .await?;
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

    /// Press on a dependency link (#357): link `index` of the range link
    /// series `series_id`, as a `pick_data` hit names it (a histogram bin of
    /// that series). The link becomes the data selection and the drag
    /// starts: `on_move` moves its exit along the predecessor's edge
    /// (`drag_link_exit`, moving a shared trunk as one and snapping to the
    /// default within 3 px) and `on_release` ends it. Returns `false` and
    /// changes nothing for an id or index that names no link.
    /// JS: `const hit = JSON.parse(await chart.pick_data(x, y, 6));`
    /// `if (hit?.kind === "HistogramBin") chart.press_range_link(hit.series_id, hit.index);`
    pub fn press_range_link(&mut self, series_id: &str, index: usize) -> Result<bool, JsValue> {
        let Some(series) = self
            .chart_series()?
            .iter()
            .find(|series| series.series_id == series_id)
        else {
            return Ok(false);
        };
        if !series
            .render_type
            .range_links()
            .is_some_and(|links| index < links.links.len())
        {
            return Ok(false);
        }
        let reference = renderer::config::PickedDataRef::HistogramBin {
            source_id: series.source_id.clone(),
            series_id: series.series_id.clone(),
            bin_index: index,
            half: None,
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

    /// Set the exit offset of link `index` of the range link series
    /// `series_id` (#357), in logical pixels from its predecessor's start;
    /// `undefined`, 0 or a non-finite value restores the default exit.
    /// Returns `false` and changes nothing for an unknown series, an index
    /// past its links or a negative offset.
    pub fn set_link_exit_offset(
        &mut self,
        series_id: &str,
        index: usize,
        offset: Option<f32>,
    ) -> Result<bool, JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let Some(links) = series
            .iter_mut()
            .find(|s| s.series_id == series_id)
            .and_then(|s| s.render_type.range_links_mut())
        else {
            return Ok(false);
        };
        if !links.set_link_exit_offset(index, offset) {
            return Ok(false);
        }
        let config = self.chart_config()?.clone();
        self.replace_chart_state(config, series)?;
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
        let Some(diagram) = series.render_type.feynman() else {
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
    pub fn place_feynman_vertices(&mut self, series_id: &str) -> Result<bool, JsValue> {
        let laid_out = self
            .renderer
            .place_feynman_vertices(self.chart_id, series_id)
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
        let Some(levels) = series.render_type.energy_levels() else {
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
                .and_then(|s| s.render_type.feynman_mut())
            else {
                return Ok(());
            };
            if diagram.drag_label(part, index, dx, dy) == NudgeResult::Moved {
                let config = self.chart_config()?.clone();
                self.replace_chart_state(config, series)?;
            }
            return Ok(());
        }
        // A dependency link moves its exit along its predecessor (#357):
        // along the value axis, x for horizontal bars and y (up) otherwise.
        if let Some(renderer::config::PickedDataRef::HistogramBin {
            series_id,
            bin_index,
            ..
        }) = self
            .chart_config()?
            .picked_data
            .as_ref()
            .and_then(|picked| picked.refs.first())
            .cloned()
            && self
                .chart_series()?
                .iter()
                .any(|s| s.series_id == series_id && s.render_type.range_links().is_some())
        {
            let (dx, dy) = self.display_delta_to_document(dx, dy)?;
            let config = self.chart_config()?.clone();
            let horizontal = config
                .bar_groups_of(config.layer_of_series(&series_id))
                .direction
                == renderer::data_config::CategoryBarDirection::Horizontal;
            let along = if horizontal { dx } else { -dy };
            let mut series = self.chart_series()?.to_vec();
            if let Some(links) = series
                .iter_mut()
                .find(|s| s.series_id == series_id)
                .and_then(|s| s.render_type.range_links_mut())
                && links.drag_link_exit(bin_index, along) == NudgeResult::Moved
            {
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
        if let (Some(part), Some(levels)) = (part, item.render_type.energy_levels_mut()) {
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
    ///
    /// Returns a number, or `undefined`.
    #[wasm_bindgen(unchecked_return_type = "number | undefined")]
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
    ///
    /// Resolves to JSON text (`FiggyJson`), not an object. `x`, `y` and
    /// `max_distance_px` are physical canvas pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson | undefined")]
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
        self.compile_pick_pipelines([x, y], max_distance_px).await?;
        self.compile_data_pick_pipelines([x, y], max_distance_px)
            .await?;
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

    /// End every element selection and pointer gesture (#311): the
    /// decoration selection (`HitId`), the annotation selection, a value
    /// label or Feynman part drag, an element drag or handle resize, a
    /// pending plot-area resize proposal and a held subplot display
    /// placement. The chart redraws. The active layer, the config, the
    /// series and the columns stay as they are, and so does the data
    /// highlight (`picked_data`, a value label's typed selection included):
    /// clear that with `set_picked_data("null")`.
    ///
    /// JS: `chart.clear_selection();`
    pub fn clear_selection(&mut self) -> Result<(), JsValue> {
        if self.chart_selection()?.is_some() {
            self.set_chart_selection(None)?;
        }
        self.set_selected_annotation(None);
        self.dragging = false;
        self.resizing = None;
        self.annotation_pointer = false;
        self.value_label_pointer = false;
        self.feynman_pointer = None;
        self.plot_resize = None;
        if self.display_held {
            self.display_held = false;
            self.renderer
                .hold_chart_display(self.chart_id, false)
                .map_err(js_err)?;
        }
        self.request_host_redraw();
        Ok(())
    }

    /// `true` while a chart element or an annotation is selected.
    pub fn has_selection(&self) -> Result<bool, JsValue> {
        Ok(self.chart_selection()?.is_some() || self.selected_annotation.is_some())
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

impl FiggyChart {
    /// The records the energy-level `series` draws, its own or its
    /// source's among the chart's series (`renderer::energy_level_records`).
    fn energy_records<'s>(
        &'s self,
        series: &'s renderer::SeriesConfig,
    ) -> Result<&'s renderer::energy_levels::EnergyLevelSeries, JsValue> {
        let levels = series
            .render_type
            .energy_levels()
            .ok_or_else(|| js_err("selected series is not an energy-level diagram"))?;
        renderer::energy_level_records(levels, self.chart_series()?)
            .ok_or_else(|| js_err("the energy-level source names no series with records"))
    }
}
