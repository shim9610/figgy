//! `FiggyChart::selection_details`: the first typed selection as JSON,
//! one method per picked family.

use super::*;

#[wasm_bindgen]
impl FiggyChart {
    /// Inspect the first resident typed selection as JSON, or `"null"` when
    /// none is selected. Only selected GPU samples are read back. Histograms
    /// expose `bin_start`, `bin_end`, `value`, and `orientation`; the value
    /// is the stored height (also valid for weighted/density histograms).
    /// Point and matrix selections expose their coordinates and cell value.
    ///
    /// Resolves to JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
            // An energy-level series is DataLess (#321): its pick names a
            // row of its records, a histogram bin a level stroke and a point
            // a transition. The row is reported as configured; no column is
            // read and no histogram path is taken.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.energy_levels().is_some() =>
            {
                self.energy_level_details(&series, bin_index, &mut result)?;
            }
            PickedDataRef::Point { point_index, .. }
                if series.render_type.energy_levels().is_some() =>
            {
                self.energy_transition_details(&series, point_index, &mut result)?;
            }
            // A monotonic series' point is a kept point of its envelope
            // (#271): its x and y are read from the envelope, not the GPU.
            PickedDataRef::Point { point_index, .. } if series.monotonic().is_some() => {
                self.monotonic_point_details(&series, point_index, &mut result)?;
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
                self.box_point_details(&series, point_index, &mut result)
                    .await?;
            }
            PickedDataRef::Point { point_index, .. } => {
                self.point_details(series, point_index, &mut result).await?;
            }
            // A box reports its category and its statistics, read back from
            // the record's GPU columns the draw used.
            PickedDataRef::HistogramBin {
                bin_index, half, ..
            } if series.render_type.box_plot().is_some() => {
                self.box_details(&series, bin_index, half, &mut result)
                    .await?;
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
                self.range_bar_details(&series, bin_index, &mut result)
                    .await?;
            }
            // A category bar reports its category and its value, read back
            // from the GPU column the draw used.
            PickedDataRef::HistogramBin { bin_index, .. }
                if series.render_type.category_bars().is_some() =>
            {
                self.category_bar_details(&series, bin_index, &mut result)
                    .await?;
            }
            PickedDataRef::HistogramBin { bin_index, .. } => {
                self.histogram_bin_details(&series, bin_index, &mut result)
                    .await?;
            }
            PickedDataRef::MatrixCell {
                x_index, y_index, ..
            } => {
                self.matrix_cell_details(&series, x_index, y_index, &mut result)
                    .await?;
            }
            // A value label reports its category, its drawn string and its
            // drag offset: configuration, nothing is read back.
            // An energy-level label (#318) lives in the series' own rows:
            // `part` names the list, `category_index` the row. It reports
            // the row's id, its label and drag offset, all configuration;
            // the categorical label text is never looked up.
            PickedDataRef::ValueLabel {
                category_index,
                part,
                ..
            } if series.render_type.energy_levels().is_some() => {
                self.energy_label_details(&series, category_index, part, &mut result)?;
            }
            PickedDataRef::ValueLabel { category_index, .. } => {
                self.value_label_details(&series, category_index, &mut result)?;
            }
            // A Feynman part (#234) reports its row from the SSOT; nothing
            // is read back. A vertex reports where it is drawn: its `at`, or
            // the layout's place while it has none (`placed: false`).
            PickedDataRef::FeynmanVertex { vertex_index, .. } => {
                self.feynman_vertex_details(&series, vertex_index, &mut result)?;
            }
            PickedDataRef::FeynmanPropagator {
                propagator_index, ..
            } => {
                self.feynman_propagator_details(&series, propagator_index, &mut result)?;
            }
            PickedDataRef::FeynmanLabel { part, index, .. } => {
                self.feynman_label_details(&series, part, index, &mut result)?;
            }
            // A mesh triangle reports its three vertex indices, read from
            // its index column's `u32` lanes, then their coordinates and its
            // values (one per vertex, or its face value), read back from
            // the GPU columns the draw used (#210).
            PickedDataRef::MeshTriangle { triangle_index, .. } => {
                self.mesh_triangle_details(&series, triangle_index, &mut result)
                    .await?;
            }
            _ => return Ok("null".into()),
        }
        serde_json::to_string(&result).map_err(js_err)
    }
}

impl FiggyChart {
    fn energy_level_details(
        &self,
        series: &SeriesConfig,
        bin_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let records = self.energy_records(&series)?;
        let level = records
            .levels
            .get(*bin_index)
            .ok_or_else(|| js_err("selected level no longer exists"))?;
        result["index"] = serde_json::json!(bin_index);
        result["part"] = serde_json::json!("level");
        result["id"] = serde_json::json!(level.id);
        result["record"] = serde_json::to_value(level).map_err(js_err)?;
        Ok(())
    }

    fn energy_transition_details(
        &self,
        series: &SeriesConfig,
        point_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let records = self.energy_records(&series)?;
        let transition = records
            .transitions
            .get(*point_index)
            .ok_or_else(|| js_err("selected transition no longer exists"))?;
        result["index"] = serde_json::json!(point_index);
        result["part"] = serde_json::json!("transition");
        result["id"] = serde_json::json!(transition.id);
        result["record"] = serde_json::to_value(transition).map_err(js_err)?;
        Ok(())
    }

    fn monotonic_point_details(
        &self,
        series: &SeriesConfig,
        point_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let point = self
            .renderer
            .monotonic_kept_point(self.monotonic_id()?, &series.series_id, *point_index)
            .map_err(js_err)?;
        result["index"] = serde_json::json!(point_index);
        result["x"] = serde_json::json!(point.map(|(x, _)| x));
        result["y"] = serde_json::json!(point.map(|(_, y)| y).filter(|y| y.is_finite()));
        Ok(())
    }

    async fn box_point_details(
        &self,
        series: &SeriesConfig,
        point_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let Some(points) = series
            .render_type
            .box_plot()
            .and_then(|b| b.points.as_ref())
        else {
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
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map_or(config, |subplot| &subplot.config)
            }
            renderer::LayerRef::Base
            | renderer::LayerRef::Overlay(_)
            | renderer::LayerRef::Annotations => config,
        };
        // The axes the layer scales by: an overlay's own or, linked,
        // its chart's.
        let (x_axis, y_axis) = panel.scaling_axes(match layer {
            renderer::LayerRef::Overlay(index) | renderer::LayerRef::SubplotOverlay(_, index) => {
                Some(index)
            }
            _ => None,
        });
        let categories = match groups.direction {
            renderer::CategoryBarDirection::Vertical => &x_axis.categories,
            renderer::CategoryBarDirection::Horizontal => &y_axis.categories,
        };
        let category = (values[0].is_finite() && values[0] >= 0.0)
            .then(|| values[0].round() as usize)
            .and_then(|k| categories.as_deref().and_then(|c| c.get(k)));
        if let Some(category) = category {
            result["category_id"] = serde_json::json!(category.id);
            result["category_label"] = serde_json::json!(category.label);
        }
        result["direction"] = serde_json::json!(groups.direction);
        Ok(())
    }

    async fn point_details(
        &self,
        series: SeriesConfig,
        point_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        // A colour- or size-mapped point also reports the value its
        // colour or radius was derived from, read back from the
        // same GPU column the draw used.
        let scatter = scatter_style(&series.render_type);
        let color_column = scatter
            .and_then(|scatter| scatter.point_color_map.as_ref())
            .map(|map| map.column.clone());
        let size_map = scatter
            .and_then(|scatter| scatter.point_size_map.as_ref())
            .cloned();
        let size_column = size_map.as_ref().map(|map| map.column.clone());
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
        if let Some(map) = &size_map {
            let size_value = values[next];
            result["size_value"] = serde_json::json!(size_value);
            // The radius the map gives this value (#312), in
            // logical pixels, the unit of `point_size`: the draw and
            // pick shaders use the same formula. A non-finite value
            // has none, and the draw skips that marker.
            let radius = map.radius_px(size_value);
            result["mapped_radius_px"] = serde_json::json!(radius);
            result["size_hidden"] = serde_json::json!(radius.is_none());
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
            if let Some(column) = &parallel.color_column {
                samples.push((column.to_owned(), *point_index));
            }
            let read = self.read_selection_samples(&samples).await?;
            let axes = parallel.columns.len();
            result["values"] = serde_json::json!(read[..axes]);
            if parallel.color_column.is_some() {
                result["color_value"] = serde_json::json!(read[axes]);
            }
        }
        Ok(())
    }

    async fn box_details(
        &self,
        series: &SeriesConfig,
        bin_index: &usize,
        half: &Option<renderer::data_config::ViolinSide>,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        // A split violin's picked half, as the selection names it.
        if let Some(half) = half {
            result["half"] = serde_json::json!(half);
        }
        let config = self.chart_config()?;
        let layer = config.layer_of_series(&series.series_id);
        let groups = config.bar_groups_of(layer);
        let panel = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map_or(config, |subplot| &subplot.config)
            }
            renderer::LayerRef::Base
            | renderer::LayerRef::Overlay(_)
            | renderer::LayerRef::Annotations => config,
        };
        // The axes the layer scales by: an overlay's own or, linked,
        // its chart's.
        let (x_axis, y_axis) = panel.scaling_axes(match layer {
            renderer::LayerRef::Overlay(index) | renderer::LayerRef::SubplotOverlay(_, index) => {
                Some(index)
            }
            _ => None,
        });
        let categories = match groups.direction {
            renderer::CategoryBarDirection::Vertical => &x_axis.categories,
            renderer::CategoryBarDirection::Horizontal => &y_axis.categories,
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
        let mut samples: [(String, usize); renderer::MAX_RECORD_FIELDS] = Default::default();
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
        Ok(())
    }

    async fn range_bar_details(
        &self,
        series: &SeriesConfig,
        bin_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        use renderer::data_config::RangeColor;
        let Some(bars) = series.render_type.range_bars() else {
            return Err(js_err("internal: the guard matched a range bar series"));
        };
        let colour = match &bars.color_by {
            RangeColor::Single => None,
            RangeColor::Direction { total, .. } => {
                total.as_ref().map(|t| ("total", t.column.as_str()))
            }
            RangeColor::State { column, .. } => Some(("state", column.as_str())),
        };
        let roles = [
            Some(("position", series.x_column.as_str())),
            Some(("end", series.y_column.as_str())),
            Some(("start", bars.start.as_str())),
            bars.whiskers.as_ref().map(|w| ("low", w.low.as_str())),
            bars.whiskers.as_ref().map(|w| ("high", w.high.as_str())),
            colour,
            bars.progress
                .as_ref()
                .map(|p| ("progress", p.column.as_str())),
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
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map_or(config, |subplot| &subplot.config)
            }
            renderer::LayerRef::Base
            | renderer::LayerRef::Overlay(_)
            | renderer::LayerRef::Annotations => config,
        };
        // The axes the layer scales by: an overlay's own or, linked,
        // its chart's.
        let (x_axis, y_axis) = panel.scaling_axes(match layer {
            renderer::LayerRef::Overlay(index) | renderer::LayerRef::SubplotOverlay(_, index) => {
                Some(index)
            }
            _ => None,
        });
        let categories = match groups.direction {
            renderer::CategoryBarDirection::Vertical => &x_axis.categories,
            renderer::CategoryBarDirection::Horizontal => &y_axis.categories,
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
        Ok(())
    }

    async fn category_bar_details(
        &self,
        series: &SeriesConfig,
        bin_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let config = self.chart_config()?;
        let layer = config.layer_of_series(&series.series_id);
        let groups = config.bar_groups_of(layer);
        let panel = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map_or(config, |subplot| &subplot.config)
            }
            renderer::LayerRef::Base
            | renderer::LayerRef::Overlay(_)
            | renderer::LayerRef::Annotations => config,
        };
        // The axes the layer scales by: an overlay's own or, linked,
        // its chart's.
        let (x_axis, y_axis) = panel.scaling_axes(match layer {
            renderer::LayerRef::Overlay(index) | renderer::LayerRef::SubplotOverlay(_, index) => {
                Some(index)
            }
            _ => None,
        });
        let categories = match groups.direction {
            renderer::CategoryBarDirection::Vertical => &x_axis.categories,
            renderer::CategoryBarDirection::Horizontal => &y_axis.categories,
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
        Ok(())
    }

    async fn histogram_bin_details(
        &self,
        series: &SeriesConfig,
        bin_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let renderer::DataRenderType::Histogram(renderer::HistogramSeries { bar }) =
            &series.render_type
        else {
            return Err(js_err("selected series is not a histogram"));
        };
        let (edges, heights) = match bar.orientation {
            renderer::data_config::BarOrientation::Vertical => (&series.x_column, &series.y_column),
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
        Ok(())
    }

    async fn matrix_cell_details(
        &self,
        series: &SeriesConfig,
        x_index: &usize,
        y_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        use renderer::data_config::{GridLayout, MatrixOrientation};
        // A field's matrix, or a grid vector's `u` grid with its `v`
        // grid and colour grid beside it.
        let image = series.render_type.rgb_image();
        let (matrix, vector) = match &series.render_type {
            DataRenderType::Field(renderer::data_config::FieldSeries { matrix, .. }) => {
                (matrix, None)
            }
            // An image's lattice is its red grid's; the green and
            // blue values are read beside it (#139).
            DataRenderType::RgbImage(RgbImageSeries { red, .. }) => (red, None),
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
        Ok(())
    }

    fn energy_label_details(
        &self,
        series: &SeriesConfig,
        category_index: &usize,
        part: &Option<renderer::energy_levels::LabelPart>,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        use renderer::energy_levels::LabelPart;
        let levels = self.energy_records(&series)?;
        let part = part.ok_or_else(|| js_err("an energy-level label names its part"))?;
        let index = *category_index;
        let (id, label) = match part {
            LabelPart::Level => levels.levels.get(index).map(|r| (&r.id, &r.label)),
            LabelPart::Transition => levels.transitions.get(index).map(|r| (&r.id, &r.label)),
            LabelPart::Limit => levels.limits.get(index).map(|r| (&r.id, &r.label)),
            LabelPart::Band => levels.bands.get(index).map(|r| (&r.id, &r.label)),
        }
        .ok_or_else(|| js_err("selected label no longer exists"))?;
        let offset = levels
            .label_offset(part, index)
            .ok_or_else(|| js_err("selected label no longer exists"))?;
        result["index"] = serde_json::json!(index);
        result["part"] = serde_json::to_value(part).map_err(js_err)?;
        result["id"] = serde_json::json!(id);
        result["label"] = serde_json::to_value(label).map_err(js_err)?;
        result["text"] = match label {
            Some(label) => serde_json::json!(
                label
                    .segments
                    .iter()
                    .map(|segment| segment.text)
                    .collect::<String>()
            ),
            None => serde_json::Value::Null,
        };
        result["offset"] = serde_json::json!(offset);
        Ok(())
    }

    fn value_label_details(
        &self,
        series: &SeriesConfig,
        category_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let config = self.chart_config()?;
        let layer = config.layer_of_series(&series.series_id);
        let panel = match layer {
            renderer::LayerRef::Subplot(index) | renderer::LayerRef::SubplotOverlay(index, _) => {
                config
                    .subplots
                    .get(index)
                    .map_or(config, |subplot| &subplot.config)
            }
            renderer::LayerRef::Base
            | renderer::LayerRef::Overlay(_)
            | renderer::LayerRef::Annotations => config,
        };
        // The axes the layer scales by: an overlay's own or, linked,
        // its chart's.
        let (x_axis, y_axis) = panel.scaling_axes(match layer {
            renderer::LayerRef::Overlay(index) | renderer::LayerRef::SubplotOverlay(_, index) => {
                Some(index)
            }
            _ => None,
        });
        let categories = if series.render_type.radar().is_some() {
            &panel.bottom_x.categories
        } else {
            match config.bar_groups_of(layer).direction {
                renderer::CategoryBarDirection::Vertical => &x_axis.categories,
                renderer::CategoryBarDirection::Horizontal => &y_axis.categories,
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
        Ok(())
    }

    fn feynman_vertex_details(
        &self,
        series: &SeriesConfig,
        vertex_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let diagram = series
            .render_type
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
        Ok(())
    }

    fn feynman_propagator_details(
        &self,
        series: &SeriesConfig,
        propagator_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let diagram = series
            .render_type
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
        Ok(())
    }

    fn feynman_label_details(
        &self,
        series: &SeriesConfig,
        part: &renderer::feynman::FeynmanLabelPart,
        index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let diagram = series
            .render_type
            .feynman()
            .ok_or_else(|| js_err("selected series is not a Feynman diagram"))?;
        let offset = diagram
            .label_offset(*part, *index)
            .ok_or_else(|| js_err("selected label no longer exists"))?;
        result["index"] = serde_json::json!(index);
        result["part"] = serde_json::to_value(part).map_err(js_err)?;
        result["offset"] = serde_json::json!(offset);
        Ok(())
    }

    async fn mesh_triangle_details(
        &self,
        series: &SeriesConfig,
        triangle_index: &usize,
        result: &mut serde_json::Value,
    ) -> Result<(), JsValue> {
        let mesh = series
            .render_type
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
        Ok(())
    }
}
