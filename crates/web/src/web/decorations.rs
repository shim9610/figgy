//! Axes, titles, legend, colorbar, annotations, decoration bounds and
//! auto-fit.

use super::*;
use renderer::theme::ThemeScope;

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
/// The index of the one annotation whose `id` is `id` (#315): `None` when
/// none has it, an error for an empty id or one several items share.
pub(crate) fn annotation_index_of(
    items: &[renderer::Annotation],
    id: &str,
) -> Result<Option<u32>, JsValue> {
    if id.is_empty() {
        return Err(js_err("an annotation id must not be empty"));
    }
    let mut found = None;
    for (index, item) in items.iter().enumerate() {
        if item.id == id {
            if found.is_some() {
                return Err(js_err(format!(
                    "more than one annotation has the id {id:?}"
                )));
            }
            found = Some(index as u32);
        }
    }
    Ok(found)
}

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

/// The `annotation_preview` JSON of `item` (#312), shared by `FiggyChart`
/// and the radial chart (#334): `{ kind, visible, drawn, canvas, document }`
/// from the placed `(canvas, document)` shapes, or `drawn: false` and the
/// skip `reason`.
pub(crate) fn annotation_preview_value(
    item: &renderer::Annotation,
    placed: Result<(serde_json::Value, serde_json::Value), renderer::AnnotationSkip>,
) -> serde_json::Value {
    let mut value = serde_json::json!({
        "kind": annotation_kind_name(&item.kind),
        "visible": item.visible,
    });
    match placed {
        Ok((canvas, document)) => {
            value["drawn"] = true.into();
            value["canvas"] = canvas;
            value["document"] = document;
        }
        Err(reason) => {
            value["drawn"] = false.into();
            value["reason"] = format!("{reason:?}").into();
        }
    }
    value
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

/// The scope forms the theme calls take, as a `subplot_scope` error's
/// `accepted` field lists them.
const THEME_SCOPE_FORMS: [&str; 4] = ["all", "base", "[indices]", "[\"base\", indices]"];

/// The subplot indices a theme scope string names and whether it names
/// the base chart: `"all"` and `"base"` name no index, a JSON array of
/// indices names those subplots alone (`"[0,2]"`), and a JSON array that
/// holds `"base"` names the base chart and those subplots
/// (`"[\"base\",0]"`). Anything else is a `subplot_scope` error whose
/// `accepted` field lists the forms.
fn parse_theme_scope(scope: &str) -> Result<(bool, Vec<usize>), JsValue> {
    let refuse = |why: String| {
        coded_error(
            &format!(
                "subplots {scope:?} is not \"all\", \"base\", a JSON array of subplot indices \
                 (those subplots alone) or a JSON array holding \"base\" and indices (the base \
                 chart and those subplots): {why}"
            ),
            "subplot_scope",
            [("accepted", js_names(&THEME_SCOPE_FORMS).into())],
        )
    };
    let list = match scope.trim() {
        "all" | "base" => return Ok((true, Vec::new())),
        list => list,
    };
    let items: Vec<serde_json::Value> =
        serde_json::from_str(list).map_err(|error| refuse(error.to_string()))?;
    let mut base = false;
    let mut indices = Vec::new();
    for item in &items {
        match item {
            serde_json::Value::String(name) if name == "base" => base = true,
            item => match item.as_u64().and_then(|index| usize::try_from(index).ok()) {
                Some(index) => indices.push(index),
                None => return Err(refuse(format!("{item} is not \"base\" or an index"))),
            },
        }
    }
    Ok((base, indices))
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

    /// Fit every axis the chart's series use to their registered columns,
    /// in one call (`Renderer::fit_ranges`, #297): the x and y axes of the
    /// base chart, of each overlay and of each subplot and its overlays,
    /// and the z axis of a 3D chart, from the columns' cached statistics
    /// with 5% padding. A chart starts at a `0..1` placeholder range; this
    /// is the call for a first plot. Synchronous, and independent of the
    /// active layer. A layer without series keeps its ranges. Then the
    /// scale of every auto-scaled vector series is fitted to the spacing
    /// of its arrows and its `auto_scale` flag consumed (#357), so the
    /// stored series say how long the arrows draw. A streamed source and a
    /// monotonic chart are refused, and a refusal changes nothing.
    /// `auto_fit_all(padding)` instead measures the active layer's drawn
    /// extents (error bar ends included) on the GPU.
    /// JS: `chart.fit_ranges()`
    pub fn fit_ranges(&mut self) -> Result<(), JsValue> {
        let mut chart = Chart::new(self.chart_config()?.clone());
        let mut series = self.chart_series()?.to_vec();
        self.renderer
            .fit_ranges(&mut chart, &mut series)
            .map_err(js_err)?;
        self.replace_chart_state(chart.config().clone(), series)?;
        Ok(())
    }

    /// Fit a candidate config without installing it (#312): `config_json`
    /// is a whole `Config` (what `get_config` returns, edited); the result
    /// is that config as JSON with the ranges of every layer, link and
    /// subplot fitted from the resident columns' statistics by the same
    /// code as `fit_ranges` (`Renderer::fit_config`), with `padding` (the
    /// fraction of the data span left on each side, finite and at least 0)
    /// instead of the fixed 5%. The fit uses this chart's registered
    /// series. The installed chart, its view and its columns are
    /// untouched; install the result with `set_config`.
    ///
    /// Refused (an exception, nothing changes): JSON that is not a
    /// `Config`, a negative or non-finite padding, a candidate whose
    /// overlays, subplots or line offsets name a series this chart has not
    /// registered (`LayerSeriesUnknown`) or break their rules, a series
    /// whose column is not registered or is streamed (rule 8), and a
    /// monotonic chart. The call is synchronous, so the columns and series
    /// it reads cannot change while it runs.
    /// JS: `const fitted = JSON.parse(chart.fit_config(JSON.stringify(cfg), 0.05));`
    pub fn fit_config(&self, config_json: &str, padding: f64) -> Result<String, JsValue> {
        let candidate: renderer::Config = serde_json::from_str(config_json).map_err(js_err)?;
        let fitted = self
            .renderer
            .fit_config(&candidate, self.chart_series()?, padding)
            .map_err(js_err)?;
        serde_json::to_string(&fitted).map_err(js_err)
    }

    /// `fit_config` that moves only Y axes (#312): every X axis of the
    /// candidate (`bottom_x`, `top_x`, each overlay's `x_axis`, the same in
    /// each subplot) comes back exactly as given, ticks and label format
    /// included; the Y axes (`left_y`, `right_y` — the z axis of a 3D
    /// chart —, overlay `y_axis` and parallel axes) are fitted. An axis no
    /// fit moves (categorical, ternary) keeps its range. The config has no
    /// "manual range" flag: a host that holds a Y range by hand restores it
    /// in the result. Same refusals as `fit_config`.
    /// JS: `chart.fit_y_axes_config(JSON.stringify(cfg), 0.05)`
    pub fn fit_y_axes_config(&self, config_json: &str, padding: f64) -> Result<String, JsValue> {
        let candidate: renderer::Config = serde_json::from_str(config_json).map_err(js_err)?;
        let fitted = self
            .renderer
            .fit_y_axes_config(&candidate, self.chart_series()?, padding)
            .map_err(js_err)?;
        serde_json::to_string(&fitted).map_err(js_err)
    }

    /// `fit_config` for one layer's chosen axes (#334,
    /// `Renderer::fit_config_scoped`): the candidate comes back with only
    /// the axes `scope_json` selects fitted, every other axis exactly as
    /// given. `scope_json` is `{ "layer": <layer>, "axes": ["x", "y",
    /// "z"], "linked": "group" | "target", "keep": [{ "layer": <layer>,
    /// "axis": "x" | "y" | "z" }] }`, a layer written as for
    /// `set_active_layer` (`"Base"`, `{"Overlay":0}`, `{"Subplot":1}`,
    /// `{"SubplotOverlay":[1,0]}`). `"z"` is a 3D chart's z axis (its
    /// `right_y`); on a parallel chart the parallel axes are `"y"`. A
    /// moved axis that is linked moves its link group. `linked` (default
    /// `"group"`) reads every series drawn against a linked axis' group,
    /// or with `"target"` only those drawn in the target's panel (the
    /// parent chart or one subplot, its overlays included; the group still
    /// shares the range). `keep` (default none) names axes that hold their
    /// range, and every axis linked to one holds too. Statistics only:
    /// error bar ends, heatmap cell half-widths and arrow tips are not
    /// included (`auto_fit_all` measures drawn extents). The installed
    /// chart is untouched.
    ///
    /// Refused (an exception, nothing changes): the refusals of
    /// `fit_config`, a scope that is not this JSON (an unknown field, axis
    /// name or `linked` value), and a `layer` or `keep` layer naming an
    /// overlay or subplot the candidate does not have (code
    /// `FitScopeLayer`).
    /// JS: `chart.fit_config_scoped(JSON.stringify(cfg), JSON.stringify({ layer: "Base", axes: ["x"], linked: "target" }), 0.05)`
    pub fn fit_config_scoped(
        &self,
        config_json: &str,
        scope_json: &str,
        padding: f64,
    ) -> Result<String, JsValue> {
        let candidate: renderer::Config = serde_json::from_str(config_json).map_err(js_err)?;
        let scope = crate::fit_scope_from_json(scope_json).map_err(js_err)?;
        let fitted = self
            .renderer
            .fit_config_scoped(&candidate, self.chart_series()?, padding, &scope.scope())
            .map_err(js_err)?;
        serde_json::to_string(&fitted).map_err(js_err)
    }

    /// Fit series `series_id`'s point size map domain to its size column's
    /// native extent, from the column's cached statistics
    /// (`PointSizeMap::fit_domain`, #312): only that series' `domain_min`
    /// and `domain_max` change, and no column is read back. Refused
    /// (an exception, the style unchanged): an unknown series, a series
    /// without `point_size_map`, a streamed or unregistered size column,
    /// and an extent that is empty, non-finite or a single value.
    /// JS: `chart.fit_point_size_domain("bubbles")`
    pub fn fit_point_size_domain(&mut self, series_id: &str) -> Result<(), JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let item = series
            .iter_mut()
            .find(|item| item.series_id == series_id)
            .ok_or_else(|| js_err("no series with this id in the chart"))?;
        self.renderer.fit_point_size_domain(item).map_err(js_err)?;
        let config = self.chart_config()?.clone();
        self.replace_chart_state(config, series)?;
        self.request_host_redraw();
        Ok(())
    }

    /// Fit vector series `series_id`'s scale so the longest vector its U/V
    /// columns' extents allow draws `target` long
    /// (`VectorStyle::fit_scale`, #312): logical pixels for a `Pixels`
    /// scale, data units for a `Data` scale. Point and grid components
    /// both fit (a grid from the union of its columns' statistics), and a
    /// point field drawn in 3D fits its `w` column too
    /// (`fit_scale_3d`). Only that series' scale value changes, and no
    /// column is read back. Refused (an exception, the style unchanged):
    /// an unknown or non-vector series, a `target` that is not finite and
    /// positive, a streamed or unregistered component column, and
    /// components whose extent is empty, non-finite or all zero.
    /// JS: `chart.fit_vector_scale("wind", 24)`
    pub fn fit_vector_scale(&mut self, series_id: &str, target: f64) -> Result<(), JsValue> {
        let mut series = self.chart_series()?.to_vec();
        let config = self.chart_config()?.clone();
        let item = series
            .iter_mut()
            .find(|item| item.series_id == series_id)
            .ok_or_else(|| js_err("no series with this id in the chart"))?;
        self.renderer
            .fit_vector_scale(&config, item, target)
            .map_err(js_err)?;
        self.replace_chart_state(config, series)?;
        self.request_host_redraw();
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
                DataRenderType::Ternary(TernarySeries {
                    scatter: Some(scatter),
                    ..
                }) => {
                    if let Some(map) = &scatter.point_color_map {
                        union(&map.column)?;
                    }
                }
                // A vector's colour map: a column or a grid's columns.
                DataRenderType::Vector(_) => match vector_columns(&cfg.render_type) {
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
                DataRenderType::Streamlines(StreamlineSeries { color_map, .. }) => {
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
                DataRenderType::Parallel(ParallelSeries { color_column, .. }) => {
                    if let Some(column) = color_column {
                        union(column)?;
                    }
                }
                // An image's colours are its data: the colourbar maps none
                // of its channels.
                DataRenderType::RgbImage(_)
                | DataRenderType::Xy(renderer::data_config::XySeries { scatter: None, .. })
                | DataRenderType::Histogram(_)
                | DataRenderType::CategoryBars(_)
                | DataRenderType::BoxPlot(_)
                | DataRenderType::RangeBars(_)
                | DataRenderType::Radar(_)
                | DataRenderType::Ternary(TernarySeries { scatter: None, .. }) => {}
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

    /// Switch the series color rotation: applies the palette to the chart
    /// (`Config::apply_palette`, #357), so series `i` takes the cycle's
    /// colour `i` in every palette-coloured part (a field, an image, an
    /// energy-level or a Feynman diagram keeps its colours), rebuilds the
    /// GPU styles, and redraws the legend symbols of the base chart, its
    /// overlays and its subplots. Later `add_*_series` calls continue the
    /// cycle.
    pub fn apply_color_cycle(&mut self, cycle: ColorCycle) -> Result<(), JsValue> {
        let cycle = renderer::ColorCycle::from(cycle);
        let mut series = self.chart_series()?.to_vec();
        let mut config = self.chart_config()?.clone();
        // The SSOT palette method (#357): series `i` takes colour `i`, and
        // the legends derived from the series follow.
        config.apply_palette(&mut series, cycle.colors());
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(js_err)?;
        let count = series.len();
        self.replace_chart_state(config, series)?;
        self.cycle = cycle;
        self.color_seq = count;
        Ok(())
    }

    /// Explicitly rebuild the auto legend from `SeriesConfig.label`: every
    /// auto legend row from the series' label; a series without one gets no
    /// row.
    pub fn reset_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
        let series = self.chart_series()?;
        let mut entries: Vec<(SeriesConfig, RichText)> = Vec::new();
        entries.try_reserve(series.len()).map_err(js_err)?;
        for cfg in series {
            if let Some(label) = &cfg.label {
                entries.push((cfg.clone(), label.clone()));
            }
        }

        let mut config = self.chart_config()?.clone();
        config.legend.content.segments.clear();
        for (cfg, label) in entries {
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(
                    &cfg,
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
                label.segments,
            );
        }
        config.legend.visible = !config.legend.content.segments.is_empty();
        self.replace_chart_config(config)?;
        self.legend_auto_managed = true;
        Ok(())
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// `x`, `y` are physical canvas pixels (the canvas's `width`/`height`
    /// space, not CSS pixels).
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

    /// The index of the selected annotation, or `undefined`. An index is
    /// valid until the annotation list changes (see `annotation_index`).
    pub fn selected_annotation(&self) -> Option<u32> {
        self.selected_annotation.map(|i| i as u32)
    }

    /// The `id` of every annotation, in index order, as an array of
    /// strings (not a JSON string): `""` for an item without one (#315).
    /// An id is the host's free label (`Annotation.id`), kept by every edit
    /// and by serialization, so it survives reordering and removal of other
    /// items, which shift indices.
    ///
    /// JS: `const ids = chart.annotation_ids(); // ["peak", "", "note-1"]`
    #[wasm_bindgen(unchecked_return_type = "string[]")]
    pub fn annotation_ids(&self) -> Result<js_sys::Array, JsValue> {
        let array = js_sys::Array::new();
        for item in &self.chart_config()?.annotations.items {
            array.push(&JsValue::from_str(&item.id));
        }
        Ok(array)
    }

    /// The current index of the annotation whose `id` is `id`, or
    /// `undefined` when none has it (#315). The renderer does not require
    /// ids to be unique, so an empty id, or one that several items share,
    /// is rejected with an error rather than guessed. Look the index up
    /// again after any edit that adds, removes or reorders annotations.
    ///
    /// JS: `const i = chart.annotation_index("peak"); // undefined when absent`
    pub fn annotation_index(&self, id: &str) -> Result<Option<u32>, JsValue> {
        annotation_index_of(&self.chart_config()?.annotations.items, id)
    }

    /// Select the annotation whose `id` is `id`, like `select_annotation`
    /// with its index (#315). Errors when no annotation, or more than one,
    /// has that id, or when `id` is empty; the selection is unchanged then.
    ///
    /// JS: `chart.select_annotation_by_id("peak");`
    pub fn select_annotation_by_id(&mut self, id: &str) -> Result<(), JsValue> {
        let index = annotation_index_of(&self.chart_config()?.annotations.items, id)?
            .ok_or_else(|| js_err(format!("no annotation has the id {id:?}")))?;
        self.select_annotation(Some(index))
    }

    /// The compass name (`"W"`, `"E"`, ...) of the annotation's handle
    /// under a point, or none. Only the handles `annotations` lists are
    /// live: a span's two edges, a line's two value handles, an arrow's
    /// two endpoints.
    ///
    /// `x`, `y` are physical canvas pixels. Returns a plain string, not JSON.
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
    ///
    /// `x`, `y` are physical canvas pixels. Returns a plain string, not JSON.
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn annotation_warnings(&self) -> Result<String, JsValue> {
        annotation_warnings_json(self.view.render_warnings())
    }

    /// Renderer-measured decoration bounds in physical canvas pixels.
    /// An `"annotation_<n>"` id that `hit_test` reports resolves to the
    /// drawn item's selection box.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// `{ rect: [x, y, width, height], handles: [[x, y, width, height], …] }`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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
                    // A polar category label's ring of angle labels (#309):
                    // the dashed circle drawn with the selection, grabbed
                    // to move every label together.
                    if let Some((center, radius)) = b.ring() {
                        value["ring"] = serde_json::json!({ "center": center, "radius": radius });
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
        check_subplot(index, config.subplots.len())?;
        let area = renderer::layout::ChartArea(Rect {
            x,
            y,
            width,
            height,
        });
        config.set_subplot_area(index, area).map_err(|error| {
            coded_error(
                &format!(
                    "subplot {index} cannot take a {width}x{height} frame ({error:?}): its plot \
                     would be empty; give it a larger frame"
                ),
                "subplot_area",
                [("index", JsValue::from(index as f64))],
            )
        })?;
        self.replace_chart_config(config)
    }

    /// Subplot `index`'s frame as JSON `{"x":..,"y":..,"width":..,"height":..}`
    /// in document pixels.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn subplot_area(&self, index: usize) -> Result<String, JsValue> {
        let subplots = &self.chart_config()?.subplots;
        check_subplot(index, subplots.len())?;
        let subplot = subplots
            .get(index)
            .ok_or_else(|| js_err(format!("no subplot {index}")))?;
        serde_json::to_string(&subplot.config.chart_area.0).map_err(js_err)
    }

    /// Fill the chart area with a colour under everything the chart draws
    /// (JSON `{"r":1,"g":1,"b":1,"a":1}`), or leave it transparent (`null`)
    /// so the page or the export's clear colour shows through (#265).
    pub fn set_background(&mut self, json: &str) -> Result<(), JsValue> {
        let color: Option<Color> = serde_json::from_str(json).map_err(|error| {
            coded_error(
                &format!(
                    "background {json:?} is not a colour JSON \
                     {{\"r\":..,\"g\":..,\"b\":..,\"a\":..}} or null: {error}"
                ),
                "background",
                [],
            )
        })?;
        let mut config = self.chart_config()?.clone();
        config.background = color;
        self.replace_chart_config(config)
    }

    /// Apply the theme family `name` (#291, #295: `gallery`, `cool-slate`,
    /// `midnight` or `warm-paper`, as [`Self::theme_names`] lists them) to
    /// the whole document: the chart, its overlays, every subplot and the
    /// series drawn there, then fit the layout to the theme's text sizes,
    /// as Rust's `Chart::apply_theme` does. The chart and every subplot
    /// take the family's member for their own kind, so subplots of other
    /// kinds take it too (`Theme::apply_named`). Ranges, text, annotations,
    /// the layer structure and the selection stay. A name that is not a
    /// family (`code` `"theme_unknown_name"`, with the family `names`)
    /// changes nothing and errors. A chart with monotonic series keeps its
    /// layout (#271): `fit_layout`, which takes the groups' sources, fits
    /// it. [`Self::apply_theme_to`] chooses the subplots.
    pub fn apply_theme(&mut self, name: &str) -> Result<(), JsValue> {
        self.apply_theme_to(name, "all")
    }

    /// [`Self::apply_theme`] to the charts `scope` names, as Rust's
    /// `Chart::apply_theme_to` takes a `ThemeScope`: `"all"` (the base chart
    /// and every subplot), `"base"` (the base chart alone), a JSON array of
    /// indices (those subplots alone, `"[0,2]"`) or a JSON array holding
    /// `"base"` (the base chart and those subplots, `"[\"base\",0]"`). A
    /// scope of another form (`code` `"subplot_scope"`, with the `accepted`
    /// forms) or an index past the subplots (`code` `"subplot_index"`)
    /// changes nothing and errors.
    pub fn apply_theme_to(&mut self, name: &str, scope: &str) -> Result<(), JsValue> {
        self.write_theme(scope, |config, series, scope| {
            renderer::theme::Theme::apply_named(name, config, series, scope)
        })
    }

    /// The member of the family `name` the chart takes (#295), as a plain
    /// object (`Theme`'s fields, colours as `{r, g, b, a}`). Edit it and
    /// pass it to [`Self::apply_custom_theme`]. A name that is not a family
    /// throws an `Error` with `code` `"theme_unknown_name"` and the family
    /// `names`.
    ///
    /// Returns a plain object, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "FiggyThemeObject")]
    pub fn theme(&self, name: &str) -> Result<JsValue, JsValue> {
        let theme = renderer::theme::Theme::for_chart(
            name,
            self.chart_config()?,
            self.chart_series()?,
            renderer::LayerRef::Base,
        )
        .map_err(theme_error)?;
        theme_to_js(&theme)
    }

    /// [`Self::theme`] with its series palette replaced by the preset
    /// `cycle` (#357): the family's chrome and parts with, say, the
    /// Okabe–Ito or ColorBrewer Set2 colours. Pass it to
    /// [`Self::apply_custom_theme`]. Refused as [`Self::theme`] refuses the
    /// name.
    ///
    /// Returns a plain object, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "FiggyThemeObject")]
    pub fn theme_with_palette(&self, name: &str, cycle: ColorCycle) -> Result<JsValue, JsValue> {
        let theme = renderer::theme::Theme::for_chart(
            name,
            self.chart_config()?,
            self.chart_series()?,
            renderer::LayerRef::Base,
        )
        .map_err(theme_error)?
        .with_palette(renderer::ColorCycle::from(cycle));
        theme_to_js(&theme)
    }

    /// The member of the family `name` subplot `index` takes (#295), as
    /// [`Self::apply_theme`] gives it there, as a plain object to edit and
    /// pass to [`Self::apply_custom_theme`]. Refused as [`Self::theme`]
    /// refuses the name, and with `code` `"subplot_index"` for an index
    /// past the subplots.
    ///
    /// Returns a plain object, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "FiggyThemeObject")]
    pub fn subplot_theme(&self, index: usize, name: &str) -> Result<JsValue, JsValue> {
        let config = self.chart_config()?;
        check_subplot(index, config.subplots.len())?;
        let theme = renderer::theme::Theme::for_chart(
            name,
            config,
            self.chart_series()?,
            renderer::LayerRef::Subplot(index),
        )
        .map_err(theme_error)?;
        theme_to_js(&theme)
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

    /// [`Self::apply_custom_theme`] to the charts `scope` names (the forms
    /// [`Self::apply_theme_to`] takes): `"[0]"` themes subplot 0 alone,
    /// whatever the base chart's kind. A chart in scope of another kind
    /// than the theme throws `"theme_wrong_kind"`, an index past the
    /// subplots `"subplot_index"`.
    pub fn apply_custom_theme_to(&mut self, theme: JsValue, scope: &str) -> Result<(), JsValue> {
        let theme = theme_from_js(&theme)?;
        self.write_theme(scope, |config, series, scope| {
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
            ThemeScope,
        ) -> Result<(), renderer::theme::ThemeError>,
    ) -> Result<(), JsValue> {
        let (base, indices) = parse_theme_scope(subplots)?;
        let scope = match (subplots.trim(), base) {
            ("all", _) => ThemeScope::All,
            ("base", _) => ThemeScope::Base,
            (_, true) => ThemeScope::BaseAnd(&indices),
            (_, false) => ThemeScope::Subplots(&indices),
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

    /// The theme names (#291, #295), as an array of strings: the four
    /// families, `gallery`, `cool-slate`, `midnight` and `warm-paper`.
    ///
    /// Returns an array of strings, not a JSON string.
    #[wasm_bindgen(unchecked_return_type = "string[]")]
    pub fn theme_names(&self) -> js_sys::Array {
        js_names(renderer::theme::ThemeFamily::NAMES)
    }

    /// Fill subplot `index`'s plot with a colour (JSON `{"r":1,"g":1,"b":1,
    /// "a":1}`) or leave it transparent (`null`): an inset usually covers
    /// the parent's data under it.
    pub fn set_subplot_background(&mut self, index: usize, json: &str) -> Result<(), JsValue> {
        check_subplot(index, self.chart_config()?.subplots.len())?;
        let color: Option<Color> = serde_json::from_str(json).map_err(|error| {
            coded_error(
                &format!(
                    "subplot background {json:?} is not a colour JSON \
                     {{\"r\":..,\"g\":..,\"b\":..,\"a\":..}} or null: {error}"
                ),
                "subplot_background",
                [],
            )
        })?;
        let mut config = self.chart_config()?.clone();
        let subplot = config
            .subplots
            .get_mut(index)
            .ok_or_else(|| js_err(format!("no subplot {index}")))?;
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
                return Err(coded_error(
                    &format!("subplot text mode {other:?} is not \"Auto\" or \"Fixed\""),
                    "subplot_text_mode",
                    [("accepted", js_names(&["Auto", "Fixed"]).into())],
                ));
            }
        };
        let mut config = self.chart_config()?.clone();
        check_subplot(index, config.subplots.len())?;
        let subplot = config
            .subplots
            .get_mut(index)
            .ok_or_else(|| js_err(format!("no subplot {index}")))?;
        subplot.text_scale = text_scale;
        self.replace_chart_config(config)
    }

    /// Subplot `index`'s text mode (#292, #295), as its setter
    /// [`Self::set_subplot_text_scale`] takes it: `"Auto"` or `"Fixed"`.
    ///
    /// Returns a plain string, not JSON.
    #[wasm_bindgen(unchecked_return_type = "\"Auto\" | \"Fixed\"")]
    pub fn subplot_text_scale(&self, index: usize) -> Result<String, JsValue> {
        let subplots = &self.chart_config()?.subplots;
        check_subplot(index, subplots.len())?;
        let subplot = subplots
            .get(index)
            .ok_or_else(|| js_err(format!("no subplot {index}")))?;
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
        check_subplot(index, config.subplots.len())?;
        Ok(config.subplot_text_factor(index))
    }

    /// The subplots that are insets now (nested in another panel's plot),
    /// bottom to top, as a JSON array of indices.
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
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

// ------------------------------------------------------------------
// Editor queries (#312): read-only answers from the renderer's own
// placement, layout and legend code. None of them changes the document,
// the selection, the undo state or the raster.
// ------------------------------------------------------------------

impl FiggyChart {
    /// The installed document as the canvas shows it now.
    fn with_shown_document<T>(
        &self,
        read: impl FnOnce(&ShownDocument<'_>) -> Result<T, JsValue>,
    ) -> Result<T, JsValue> {
        let logical = self.chart_config()?;
        let (placed, _, scale) = self.display_config()?;
        read(&ShownDocument {
            logical,
            placed: &placed,
            scale,
        })
    }

    /// An `Annotation` JSON value, checked by the rules a document's
    /// annotation layer is. The radial chart's `annotation_preview` reads
    /// its item through it too (#334).
    pub(crate) fn parse_annotation(json: &str) -> Result<renderer::Annotation, JsValue> {
        let item: renderer::Annotation = serde_json::from_str(json).map_err(|error| {
            coded_error(
                &format!("annotation is not an Annotation JSON value: {error}"),
                "annotation_json",
                [],
            )
        })?;
        item.validate().map_err(|kind| {
            coded_error(
                &format!("annotation is not valid: {kind:?}"),
                "annotation_invalid",
                [("rule", JsValue::from_str(&format!("{kind:?}")))],
            )
        })?;
        Ok(item)
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// Where an annotation that is not part of the document yet would be
    /// drawn (#312), placed by the annotation layer's own placement code
    /// from the document as the canvas shows it: the document, the
    /// selection, the undo state and the raster stay as they are. `json` is
    /// an `Annotation`, the shape `add_annotation` takes.
    ///
    /// Returns JSON `{ kind, visible, drawn, canvas, document }`. `canvas`
    /// is the shape in physical canvas pixels and `document` in document
    /// pixels, each `{ shape, points | rect, bounds }`: `shape` is `"line"`
    /// (a reference line's two ends), `"span"` or `"text"` (`rect`
    /// `[x, y, w, h]`), `"arrow"` (`from`, `to`) or `"polygon"` (a shape's
    /// outline clipped to the plot), and `bounds` is everything the raster
    /// paints for it, labels included. A span or a reference line on a
    /// broken axis is reported whole, before its gaps are cut out. An item
    /// the raster would skip has `drawn: false` and the `reason`
    /// `annotation_warnings` would give (`NonFinite`, `NoValueAxis`,
    /// `OutOfRange`, `EmptyText`). `visible: false` (a hidden item) is
    /// placed as it is drawn once shown.
    /// JS: `const p = JSON.parse(chart.annotation_preview(itemJson));`
    pub fn annotation_preview(&self, json: &str) -> Result<String, JsValue> {
        let item = Self::parse_annotation(json)?;
        let value = self.with_shown_document(|shown| {
            let placed = shown.annotation_preview(&item).map(|(canvas, document)| {
                (
                    annotation_preview_json(&canvas, 1.0),
                    document
                        .as_ref()
                        .map_or(serde_json::Value::Null, |d| annotation_preview_json(d, 1.0)),
                )
            });
            Ok(annotation_preview_value(&item, placed))
        })?;
        serde_json::to_string(&value).map_err(js_err)
    }

    /// `json`, an `Annotation`, with every anchor converted to
    /// `anchor_kind` (`"Data"` or `"Ratio"`) at the position it is drawn
    /// at now (#312): a text box's anchor, an arrow's and a box shape's
    /// two ends, a polygon's vertices. A reference line or a span has no
    /// anchor and comes back unchanged. Nothing is installed; pass the
    /// result to `add_annotation` or `set_annotation`. An anchor with no
    /// position (a data anchor on a chart without value axes, a
    /// non-positive value on a logarithmic axis, a non-finite coordinate)
    /// is refused with code `anchor_unplaceable`. A ratio anchor is an
    /// `f32` fraction of the plot, so a round trip keeps the drawn pixel.
    /// JS: `chart.set_annotation(i, chart.convert_annotation_anchor(itemJson, "Ratio"));`
    pub fn convert_annotation_anchor(
        &self,
        json: &str,
        anchor_kind: &str,
    ) -> Result<String, JsValue> {
        let kind = match anchor_kind {
            "Data" => renderer::AnchorKind::Data,
            "Ratio" => renderer::AnchorKind::Ratio,
            _ => {
                return Err(coded_error(
                    &format!("anchor kind must be \"Data\" or \"Ratio\", got {anchor_kind:?}"),
                    "anchor_kind",
                    [("accepted", JsValue::from(js_names(&["Data", "Ratio"])))],
                ));
            }
        };
        let item = Self::parse_annotation(json)?;
        let converted = self.with_shown_document(|shown| {
            item.converted_anchors(&shown.annotation_space(), kind)
                .ok_or_else(|| {
                    coded_error(
                        &format!(
                            "an anchor of this {} has no position on the chart as drawn, so it \
                             cannot be expressed as {anchor_kind}",
                            annotation_kind_name(&item.kind)
                        ),
                        "anchor_unplaceable",
                        [],
                    )
                })
        })?;
        serde_json::to_string(&converted).map_err(js_err)
    }

    /// The displayed geometry of a document and its panels, read-only
    /// (#312). Without a config it is the installed document as the canvas
    /// shows it; with a `Config` JSON it is that candidate, placed on this
    /// canvas the way `set_config` would place it, and nothing is
    /// installed. A candidate whose overlays, subplots or line offsets
    /// cannot be laid out is refused with code `layout`.
    ///
    /// `surface_width` and `surface_height` (#334), physical pixels, lay
    /// the document (or the candidate) out as a canvas of that size would
    /// show it, the way the chart reports it after `resize(width,
    /// height)`; the canvas and its surface are not resized. Give both or
    /// neither: one alone, or a dimension that is not a whole number of
    /// pixels from 1, is refused with code `invalid_size`. Without them the
    /// call reads the canvas as before.
    ///
    /// Returns JSON `{ scale, surface, document_area, active_layer, panels }`:
    /// `scale` is canvas pixels per document pixel, `surface` the canvas
    /// size, and every rect a `{ document, canvas }` pair of
    /// `[x, y, w, h]`, in document and physical canvas pixels. `panels`
    /// lists every panel in the order the renderer draws them (base,
    /// overlays, line offset members, then each subplot's panels), each
    /// `{ index, layer, subplot, member, frame, data_area }`: `layer` as
    /// `set_active_layer` takes it, `member` the line offset series a
    /// member panel draws, `frame` its chart's `chart_area` and
    /// `data_area` its exact data area (`null` without one). An overlay
    /// and a member share their chart's frame and data area.
    /// JS: `const layout = JSON.parse(chart.inspect_document_layout());`
    pub fn inspect_document_layout(
        &self,
        config_json: Option<String>,
        surface_width: Option<f64>,
        surface_height: Option<f64>,
    ) -> Result<String, JsValue> {
        let surface =
            crate::layout_surface(surface_width, surface_height).map_err(layout_query_error)?;
        let value = match (config_json, surface) {
            (None, None) => {
                self.with_shown_document(|shown| Ok(shown.layout_json(self.surface_size)))?
            }
            (None, Some(surface)) => crate::fitted_layout_json(self.chart_config()?, surface),
            (Some(json), surface) => crate::fitted_layout_json(
                &crate::layout_candidate(&json).map_err(layout_query_error)?,
                surface.unwrap_or(self.surface_size),
            ),
        };
        serde_json::to_string(&value).map_err(js_err)
    }

    /// The panel on top at physical canvas point `(x, y)` (#312), as JSON
    /// `{ layer, panel, subplot, active, in_data_area, document }`, or
    /// `null` off every frame. Subplot frames win over the parent chart:
    /// the one drawn on top (insets in paint order, then subplots of their
    /// own, later over earlier), the same rule a base-layer press uses.
    /// Where the active layer is an overlay of the chart found, the
    /// overlay is reported, since it is drawn over its chart's frame; the
    /// annotation layer, while active and shown, is reported on the parent
    /// frame outside every subplot. `panel` indexes
    /// `inspect_document_layout`'s `panels`, `active` says whether `layer`
    /// is the active layer, `in_data_area` whether the point is inside that
    /// chart's exact data area, and `document` is the point in document
    /// pixels.
    /// JS: `const hit = JSON.parse(chart.panel_at(x, y));`
    pub fn panel_at(&self, x: f32, y: f32) -> Result<String, JsValue> {
        let value = self.with_shown_document(|shown| Ok(shown.panel_at_json(x, y)))?;
        serde_json::to_string(&value).map_err(js_err)
    }

    /// The active layer's exact data area (#312), as JSON `{ layer, panel,
    /// document, canvas }` with `[x, y, w, h]` rects in document and
    /// physical canvas pixels: the rectangle the data is drawn in, not the
    /// padded box `decoration_bounds("data_area")` selects. The annotation
    /// layer reads the parent chart's. Refused with code `data_area` for a
    /// chart without one.
    /// JS: `const [x, y, w, h] = JSON.parse(chart.data_area_rect()).canvas;`
    pub fn data_area_rect(&self) -> Result<String, JsValue> {
        let value = self.with_shown_document(|shown| {
            shown
                .data_area_json()
                .map_err(|why| coded_error(&why, "data_area", []))
        })?;
        serde_json::to_string(&value).map_err(js_err)
    }

    /// One legend row for series `series_id` as the auto legend writes it
    /// (#312): its symbol, drawn from the series' style and the chart's
    /// colourbar map and naming the series, a space, the `"\t"` column
    /// separator and the series' label. Returns the row's `RichSegment`s as
    /// a JSON array; append them to `legend.content.segments` (after a
    /// `"\n"` segment when the legend has rows) with `set_config`, and the
    /// rest of the legend keeps its text. A series without a label gets an
    /// empty label. An unknown series is refused with code `series`.
    /// JS: `const row = JSON.parse(chart.legend_entry("fit"));`
    pub fn legend_entry(&self, series_id: &str) -> Result<String, JsValue> {
        let series = self.chart_series()?;
        let cfg = series
            .iter()
            .find(|cfg| cfg.series_id == series_id)
            .ok_or_else(|| {
                coded_error(
                    &format!("no series {series_id:?}"),
                    "series",
                    [("series_id", JsValue::from_str(series_id))],
                )
            })?;
        let config = self.chart_config()?;
        let row = renderer::config::series_legend_entry(
            cfg,
            config.colorbar.as_ref().map(|bar| &bar.colormap),
        );
        serde_json::to_string(&row).map_err(js_err)
    }

    /// Every warning of the last drawn frame (#312), as a JSON array: tick
    /// settings the axis raster drew differently from how they were
    /// written, on any axis of any panel, and annotations the raster
    /// skipped. Each is `{ kind, ..., message }`: `MajorSpacingReplaced`
    /// and `LogDecadeStepAdjusted` (`axis`, `requested`, `used`; a
    /// replaced spacing also has `reason` and, for `TooManyTicks`,
    /// `limit`), `MinorCountClamped` (`axis`, `requested`, `used`) and
    /// `AnnotationSkipped` (`index`, `reason`). `axis` is the `Config`
    /// field path (`"bottom_x"`, `"overlays[0].y_axis"`,
    /// `"subplots[1].config.left_y"`), a non-finite number is `null`, and
    /// `message` is one line naming the setting and both values. A later
    /// kind has `kind: "Other"`. The list is the one `frame` last
    /// rasterized; `annotation_warnings` is its annotation part.
    /// JS: `for (const w of JSON.parse(chart.render_warnings())) console.warn(w.message);`
    pub fn render_warnings(&self) -> Result<String, JsValue> {
        let values = self
            .view
            .render_warnings()
            .iter()
            .map(render_warning_json)
            .collect::<Vec<_>>();
        serde_json::to_string(&values).map_err(js_err)
    }
}

/// A refused layout query as a JS `Error` with its `code` (#334).
fn layout_query_error(error: crate::LayoutQueryError) -> JsValue {
    coded_error(&error.message, error.code, [])
}

/// The layout `FiggyChart.inspect_document_layout(configJson)` reports for
/// a candidate `Config` JSON on a canvas whose surface is `surface_width` x
/// `surface_height` physical pixels, without a chart (#334): no renderer,
/// GPU device or surface is created, and the result is the JSON a chart
/// of that surface size reports for the same candidate: a canvas places
/// its document by its surface size alone, and the result's `scale` is
/// that fit. Refused with code `config_json` or `layout` as
/// `inspect_document_layout` refuses a candidate, and with `invalid_size`
/// for a dimension that is not a whole number of pixels from 1.
///
/// JS: `const layout = JSON.parse(inspect_config_layout(cfg, canvas.width, canvas.height));`
#[wasm_bindgen]
pub fn inspect_config_layout(
    config_json: &str,
    surface_width: f64,
    surface_height: f64,
) -> Result<String, JsValue> {
    let value = crate::config_layout_json(config_json, surface_width, surface_height)
        .map_err(layout_query_error)?;
    serde_json::to_string(&value).map_err(js_err)
}
