//! `FiggyChart`'s document: the config and series SSoT I/O and the
//! legacy document calls.

use super::*;

#[wasm_bindgen]
impl FiggyChart {
    // ---- SSoT I/O ----
    //
    // The whole option tree (`Config`) is plain data; these round-trip it
    // as JSON so a host can read it, edit anything — axis scale, tick
    // shape/length, colors, fonts, label text — and hand it back. The
    // standard flow: auto-fit first, then refine via the SSoT.
    // Full schema reference: crates/web/SCHEMA.md.

    /// Serialize the full chart option SSoT to a JSON string.
    /// JS: `const cfg = JSON.parse(chart.get_config());`
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    /// Positions and sizes in it (`chart_area`, margins, decoration offsets)
    /// are document logical pixels, not canvas pixels.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn get_config(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_config()?).map_err(js_err)
    }

    /// Replace the whole option SSoT from JSON. Marks everything dirty —
    /// the next `frame()` re-rasters the chrome and refreshes the
    /// transform, exactly like any other config edit.
    /// Use `get_config`, edit only the desired fields, then submit the
    /// complete config here for selective control. For example, writing
    /// only axis `min` / `max` preserves tick and label settings, whereas
    /// the auto-fit helpers derive those settings along with the range.
    /// This call does not auto-fit ranges or fit the title band.
    /// JS: `chart.set_config(JSON.stringify(cfg));`
    pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
        let mut new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
        let series = self.chart_series()?.to_vec();
        let legend_content_changed = self.follow_config_legend(&mut new_cfg, &series)?;
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(js_err)?;
        self.replace_chart_config(new_cfg)?;
        if legend_content_changed {
            self.legend_auto_managed = false;
        }
        Ok(())
    }

    /// Serialize the series declarations (columns, render type, styles).
    ///
    /// Returns JSON text (`FiggyJson`), not an object: parse it with
    /// `JSON.parse`.
    #[wasm_bindgen(unchecked_return_type = "FiggyJson")]
    pub fn get_series(&self) -> Result<String, JsValue> {
        serde_json::to_string(self.chart_series()?).map_err(js_err)
    }

    /// Replace the series declarations from JSON. Column references must
    /// already be registered; GPU styles are rebuilt, legend labels are
    /// kept for series ids that survive.
    ///
    /// `Contour` and `HeatmapContour` accept `contour.levels` lists with
    /// 0 through 1024 entries. A list with 1025 or more entries rejects the
    /// JSON call with a JavaScript exception; the previously committed
    /// chart config, series declarations, and GPU styles remain active.
    /// The WASM facade neither adds a smaller limit nor truncates levels.
    /// Contour labels use the same 1024 capacity for automatic and explicit
    /// placement. `spacing_px` must be finite and positive even for hidden
    /// labels or explicit anchors; rejection preserves the prior state.
    pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
        let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
        for cfg in &new_series {
            self.ensure_columns_exist(cfg)?;
        }
        self.renderer
            .preflight_chart_styles(&new_series)
            .map_err(js_err)?;
        let mut config = self.chart_config()?.clone();
        if self.legend_auto_managed {
            follow_series_legend(&mut config, &new_series);
        }
        let new_len = new_series.len();
        self.replace_chart_state(config, new_series)?;
        self.color_seq = new_len.max(self.color_seq);
        Ok(())
    }
}

/// A Feynman diagram is fixed where it is first drawn (#234): the wrapper
/// commits the layout of every unplaced vertex as the series is
/// registered, so `get_series` returns placed coordinates.
pub(in crate::web) fn place_feynman_vertices(series: &mut [SeriesConfig]) -> Result<(), JsValue> {
    for item in series {
        if let Some(diagram) = item.render_type.feynman_mut() {
            diagram.place_vertices().map_err(js_err)?;
        }
    }
    Ok(())
}

/// An auto-managed legend after the series become `new_series`, as
/// `set_series` keeps it: entries pair with series by the series their
/// symbol names (a symbol that names none pairs by position); the entries
/// whose series is gone drop, the rest are restyled, and each labeled
/// series without an entry gets one.
pub(in crate::web) fn follow_series_legend(
    config: &mut renderer::Config,
    new_series: &[SeriesConfig],
) {
    let colormap = config.colorbar.as_ref().map(|bar| &bar.colormap);
    let content = &mut config.legend.content;
    for index in (0..renderer::config::legend_entry_count(content)).rev() {
        let kept = match renderer::config::legend_entry_series_id(content, index) {
            Some(series_id) => new_series.iter().any(|cfg| cfg.series_id == series_id),
            None => index < new_series.len(),
        };
        if !kept {
            renderer::config::remove_legend_entry(content, index);
        }
    }
    renderer::config::update_legend_symbols_preserving_text(content, new_series, colormap);
    for cfg in new_series {
        if let Some(label) = &cfg.label
            && renderer::config::legend_entry_of_series(content, new_series, &cfg.series_id)
                .is_none()
        {
            renderer::config::append_legend_entry_rich(
                content,
                renderer::config::series_symbol_segments(cfg, colormap),
                label.segments.clone(),
            );
        }
    }
    config.legend.visible = !config.legend.content.segments.is_empty();
}

#[wasm_bindgen]
impl FiggyChart {
    /// Open a legacy categorical bar chart document (`CategoricalChart` JSON,
    /// what `FiggyBoundedChart` took as the `"categorical"` family) as this
    /// Cartesian chart.
    ///
    /// `Config::from_categorical` turns the document into a config with a
    /// categorical axis and `bar_groups`, one `CategoryBars` series per
    /// document series and the columns those read: the category positions
    /// (`categorical.positions`) and one value column per series
    /// (`categorical.values.<series id>`), registered as one batch. The
    /// value axis is fitted to the bars from the columns' statistics and
    /// rounded to nice ticks. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series,
    /// columns and config of a categorical document opened before
    /// are replaced, every other column stays. From here on the chart is
    /// edited through the Cartesian API (`get_config` / `set_config`,
    /// `get_series` / `set_series`, the column registry).
    ///
    /// The replace is atomic (#311): see "Document replace" on
    /// [`Self::set_radar_document`]. The ids are the documented ones for
    /// every document opened.
    ///
    /// Returns the conversion report and takes the loss policy described
    /// in "Review and apply" on [`Self::set_radar_document`] (#334).
    ///
    /// JS: `const report = JSON.parse(chart.set_categorical_document(JSON.stringify(doc), JSON.stringify({ refuse: "any" })));`
    pub fn set_categorical_document(
        &mut self,
        json: &str,
        options: Option<String>,
    ) -> Result<String, JsValue> {
        const FAMILY: &str = "categorical";
        let (document, policy) =
            read_document_call::<renderer::CategoricalChart>(FAMILY, json, options)?;
        let (converted, report) = self.convert_document_call(FAMILY, &document, &policy)?;
        let (config, series, uploads, _) = converted;
        let horizontal = config
            .bar_groups
            .as_ref()
            .is_some_and(|g| g.direction == renderer::CategoryBarDirection::Horizontal);
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            // Fit the value axis to the bars and round it to nice ticks. A
            // range fit resets the label format, so the document's is put
            // back.
            let format = if horizontal {
                config.bottom_x.label_style.format.clone()
            } else {
                config.left_y.label_style.format.clone()
            };
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.0)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let value = if horizontal {
                &mut config.bottom_x
            } else {
                &mut config.left_y
            };
            value.label_style.format = format;
            let (min, max) = (value.min, value.max);
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if value.auto_ticks(min, max, 5).is_ok() {
                let fitted = value.clone();
                let twin = if horizontal {
                    &mut config.top_x
                } else {
                    &mut config.right_y
                };
                twin.min = fitted.min;
                twin.max = fitted.max;
                twin.major_spacing = fitted.major_spacing;
                twin.minor_count = fitted.minor_count;
            }
            Ok(config)
        })?;
        Ok(report)
    }

    /// Open a legacy box plot document (`BoxPlotChart` JSON, what
    /// `FiggyBoundedChart` took as the `"boxplot"` family) as this Cartesian
    /// chart.
    ///
    /// `Config::from_boxplot` turns the document into a config with a
    /// categorical axis and a grouped `bar_groups`, one `BoxPlot` series per
    /// document series and what those read: the category positions
    /// (`boxplot.positions`, a registry column), and per series its record
    /// of box statistics (`boxplot.record.<series id>`, as `register_record`
    /// registers one) and, when it has outliers, its ragged outlier column
    /// (`boxplot.outliers.<series id>`, as `register_ragged`). The summaries
    /// upload through borrowed adapters; nothing is copied. Without a
    /// document `value_range` the value axis is fitted to the records' and
    /// outliers' statistics with a tenth of the span as padding and rounded
    /// to nice ticks. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series, columns,
    /// records and config of a box plot document opened before are
    /// replaced, every other column stays. From here on the chart is edited
    /// through the Cartesian API.
    ///
    /// The replace is atomic (#311): see "Document replace" on
    /// [`Self::set_radar_document`]. The ids are the documented ones for
    /// every document opened.
    ///
    /// Returns the conversion report and takes the loss policy described
    /// in "Review and apply" on [`Self::set_radar_document`] (#334).
    ///
    /// JS: `const report = JSON.parse(chart.set_boxplot_document(JSON.stringify(doc), JSON.stringify({ refuse: "any" })));`
    pub fn set_boxplot_document(
        &mut self,
        json: &str,
        options: Option<String>,
    ) -> Result<String, JsValue> {
        const FAMILY: &str = "boxplot";
        let (document, policy) =
            read_document_call::<renderer::BoxPlotChart>(FAMILY, json, options)?;
        let (converted, report) = self.convert_document_call(FAMILY, &document, &policy)?;
        let (config, series, uploads, _) = converted;
        let horizontal = document.direction == renderer::BoxPlotDirection::Horizontal;
        let fit_range = document.value_range.is_none();
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            if !fit_range {
                return Ok(config);
            }
            let format = if horizontal {
                config.bottom_x.label_style.format.clone()
            } else {
                config.left_y.label_style.format.clone()
            };
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.1)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let value = if horizontal {
                &mut config.bottom_x
            } else {
                &mut config.left_y
            };
            value.label_style.format = format;
            let (min, max) = (value.min, value.max);
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if value.auto_ticks(min, max, 5).is_ok() {
                let fitted = value.clone();
                let twin = if horizontal {
                    &mut config.top_x
                } else {
                    &mut config.right_y
                };
                twin.min = fitted.min;
                twin.max = fitted.max;
                twin.major_spacing = fitted.major_spacing;
                twin.minor_count = fitted.minor_count;
            }
            Ok(config)
        })?;
        Ok(report)
    }

    /// Open a legacy radar document (`RadarChart` JSON, what
    /// `FiggyBoundedChart` took as the `"radar"` family) as this polar chart.
    ///
    /// `Config::from_radar` turns the document into a polar config whose
    /// angle axis holds the document's categories, one `Radar` series per
    /// document series and the columns those read: the category indices
    /// (`radar.positions`) and one value column per series
    /// (`radar.values.<series id>`). The values upload through borrowed
    /// adapters; nothing is copied. Without a document `value_range` the
    /// radius axis is fitted to the value columns' statistics, from zero
    /// when no value is negative (as the former chart fitted it), and
    /// rounded to nice ticks; a document `ring_spacing` is the ring spacing
    /// either way. The chart keeps its chart area, and an explicit
    /// `decorations.data_area` (logical pixels, edges rounded to whole
    /// pixels) is the data area in it (`Config::fit_legacy_data_area`, #334)
    /// unless the layout cannot hold it, which keeps the automatic layout; the series, columns and
    /// config of a radar document opened before are replaced, every other
    /// column stays. From here on the chart is edited through the Cartesian
    /// API (`get_config` / `set_config`, `get_series` / `set_series`, the
    /// column registry), and its vertices pick, select and step as points.
    ///
    /// # Review and apply
    ///
    /// The three document calls convert the document once, on this chart's
    /// chart area (`Config::from_*_with_report_in`, #334), and install that
    /// conversion. They return its report as JSON, `{config, series,
    /// uploads, columns, notes}`, which is what `convert_*_document` returns
    /// with `{"chart_area": [x, y, width, height]}` set to this chart's
    /// chart area. The report's `config` is the converted one; the
    /// installed config also has its value axis fitted (`get_config`).
    /// `options` is an optional JSON object `{"refuse"?: ["path", ...] |
    /// "any"}`: when a note's path is a listed path or lies under one
    /// (followed by `.` or `[`), or with `"any"` when there is any note,
    /// the call rejects at `"validate"` with `code` `"conversion_loss"` and
    /// the refused notes as `notes`, before anything is installed. Other
    /// option fields, or another `refuse`, reject at `"validate"` with
    /// `code` `"invalid_options"`.
    ///
    /// # Document replace
    ///
    /// The three document calls replace the chart atomically (#311), in
    /// one renderer transaction. They prepare the candidate config, series
    /// and uploads, check them (styles; each documented id is free or holds
    /// what the document uploads under it: a column, a record or a ragged
    /// column, and is not streamed), then upload every column, record and
    /// ragged column under its documented id (`radar.positions`,
    /// `boxplot.record.<series id>`, ...), fit the axes from the candidate
    /// columns, and commit the columns, the removal of the previous
    /// document's columns and the chart's config and series in one step.
    /// The previous document's columns are each id under `categorical.`,
    /// `boxplot.` or `radar.` that the replaced series read, of whichever
    /// family, that the new document does not upload again; an id it does
    /// upload again is replaced in place. The ids are therefore the
    /// documented ones for every document opened.
    /// On a chart that shows a monotonic document
    /// (`set_monotonic_document`), the document replaces the general chart
    /// behind it and the monotonic chart ends with the commit, as
    /// `clear_monotonic_document` ends it; a failure leaves the monotonic
    /// document shown.
    ///
    /// A failure rejects with an `Error` whose `stage` is `"validate"`
    /// (parse, validation, styles, ids), `"upload"` (a column that the GPU or the memory budget cannot hold, with the `code` and byte fields of
    /// `register_column_f32`), `"fit"` or `"commit"` (the series checks
    /// against the candidate columns, the chart's revisions, the picker)
    /// and whose message names the stage and the cause. The chart's
    /// config, series, columns, element and data selections, picks and
    /// image are then exactly as they were: nothing reaches the chart
    /// before the commit, which cannot fail. Only a broken internal
    /// invariant after the commit (the monotonic chart or the chart itself
    /// gone) rejects then, with an error that names it as internal and no
    /// `stage`; the new document is shown in that case.
    ///
    /// Memory: the transaction builds a complete candidate column pool, so
    /// at the peak the GPU holds the live pool slab, the new document's
    /// staging (half of it for single-lane columns, plus their transient
    /// expansion buffer) and a candidate slab of the pool's capacity — or a
    /// grown one, twice the capacity or what the surviving columns and the
    /// new document need, when they do not fit. The previous document's
    /// columns take no room in the candidate. After the commit the former
    /// slab stays as the pool's backup when the capacity did not change.
    /// A memory budget that cannot hold the peak refuses the call at the
    /// upload stage with `code` `"budget_exceeded"`.
    ///
    /// JS: `const report = JSON.parse(chart.set_radar_document(JSON.stringify(doc), JSON.stringify({ refuse: "any" })));`
    pub fn set_radar_document(
        &mut self,
        json: &str,
        options: Option<String>,
    ) -> Result<String, JsValue> {
        const FAMILY: &str = "radar";
        let (document, policy) = read_document_call::<renderer::RadarChart>(FAMILY, json, options)?;
        let (converted, report) = self.convert_document_call(FAMILY, &document, &policy)?;
        let (config, series, uploads, _) = converted;
        let fit_range = document.value_range.is_none();
        let ring_spacing = document.ring_spacing;
        self.replace_document(FAMILY, config, series, &uploads, |pool, config, series| {
            if !fit_range {
                return Ok(config);
            }
            // Fit the radius axis to the values, from zero when none is
            // negative, and round it to nice ticks. A range fit resets the
            // label format, so the document's is put back.
            let format = config.left_y.label_style.format.clone();
            let mut fitted = renderer::Chart::new(config);
            fitted
                .auto_fit_layer_union(pool, renderer::LayerRef::Base, series, 0.0)
                .map_err(js_err)?;
            let mut config = fitted.config().clone();
            let radius = &mut config.left_y;
            radius.label_style.format = format;
            let min = if radius.min >= 0.0 { 0.0 } else { radius.min };
            let max = radius.max;
            // A fitted range that ticks cannot round (`TickError`: empty,
            // non-finite or non-positive on a log axis) keeps the fit as it is.
            if radius.auto_ticks(min, max, 5).is_ok()
                && let Some(spacing) = ring_spacing
            {
                radius.major_spacing = spacing;
            }
            Ok(config)
        })?;
        Ok(report)
    }
}

/// The id prefixes of the columns the legacy document calls register: a
/// column the replaced series read under one of them belongs to the
/// replaced document (#311).
const DOCUMENT_ID_PREFIXES: [&str; 3] = ["categorical.", "boxplot.", "radar."];

/// The borrowed adapters one document upload reads through (#311): none for
/// a plain column, which is its own source, the nine record fields of a box
/// plot series, or its outlier values and offsets.
enum DocumentAdapters<'a> {
    Column,
    Record([renderer::BoxColumn<'a>; 9]),
    Outliers([renderer::BoxColumn<'a>; 2]),
}

/// `error` with the document replace stage it failed at (#311): the message
/// names the family and the stage, and the `Error` carries `stage`; the
/// `code` and byte fields of a registration error stay on it.
pub(in crate::web) fn document_stage_error(family: &str, stage: &str, error: JsValue) -> JsValue {
    use wasm_bindgen::JsCast;
    let cause = match error.dyn_ref::<js_sys::Error>() {
        Some(error) => String::from(error.message()),
        None => error
            .as_string()
            .unwrap_or_else(|| "an error that is not a string".to_owned()),
    };
    let message = format!("{family} document {stage} failed: {cause}");
    let object = match error.dyn_into::<js_sys::Error>() {
        Ok(object) => {
            object.set_message(&message);
            object
        }
        Err(_) => js_sys::Error::new(&message),
    };
    match js_sys::Reflect::set(
        object.as_ref(),
        &JsValue::from_str("stage"),
        &JsValue::from_str(stage),
    ) {
        Ok(true) => object.into(),
        // The message names the stage either way.
        Ok(false) | Err(_) => JsValue::from_str(&message),
    }
}

/// The legacy document of a `set_*_document` call and its loss policy
/// (#334), read before anything else: options that are not
/// `{"refuse"?: [path, ...] | "any"}` reject at `"validate"` with `code`
/// `"invalid_options"`, a document that does not parse or breaks its rules
/// at `"validate"` as before.
fn read_document_call<D: crate::LegacyDocument>(
    family: &str,
    json: &str,
    options: Option<String>,
) -> Result<(D, crate::LossPolicy), JsValue> {
    let validate = |error| document_stage_error(family, "validate", document_error(error));
    let policy = crate::LossPolicy::parse(options.as_deref()).map_err(validate)?;
    let document = D::read(json).map_err(validate)?;
    Ok((document, policy))
}

impl FiggyChart {
    /// The one conversion a `set_*_document` call installs (#334):
    /// `document` converted on this chart's chart area
    /// (`Config::from_*_with_report_in`, as `convert_*_document` with that
    /// `chart_area` converts it), with its report JSON, the call's return
    /// value. When `policy` refuses a note, rejects at `"validate"` with
    /// `code` `"conversion_loss"` and the refused notes as `notes` (an
    /// array of `{path, kind, reason}`), before anything is installed.
    fn convert_document_call<'d, D: crate::LegacyDocument>(
        &self,
        family: &str,
        document: &'d D,
        policy: &crate::LossPolicy,
    ) -> Result<(renderer::ConvertedDocument<'d>, String), JsValue> {
        let validate = |error| document_stage_error(family, "validate", error);
        let chart_area = self.chart_config().map_err(validate)?.chart_area.clone();
        let converted = document.convert_in(Some(&chart_area));
        match crate::document_apply(&converted, policy).map_err(|e| validate(document_error(e)))? {
            crate::DocumentApply::Report(report) => Ok((converted, report)),
            crate::DocumentApply::Refused(notes) => {
                let notes = js_sys::JSON::parse(&notes).map_err(validate)?;
                let paths = converted
                    .3
                    .notes
                    .iter()
                    .filter(|note| policy.refuses(note))
                    .fold(String::new(), |mut paths, note| {
                        if !paths.is_empty() {
                            paths.push_str(", ");
                        }
                        paths.push_str(&note.path);
                        paths
                    });
                Err(validate(coded_error(
                    &format!("the conversion would drop or change refused fields: {paths}"),
                    "conversion_loss",
                    [("notes", notes)],
                )))
            }
        }
    }
}

impl FiggyChart {
    /// Whether `id` names anything the renderer holds: a column (resident or
    /// streamed), a record, a ragged column or a triangle index column.
    fn document_id_in_use(&self, id: &str) -> bool {
        let pool = self.renderer.pool();
        self.renderer.logical_column(id).is_some()
            || pool.record(id).is_some()
            || pool.ragged(id).is_some()
            || pool.triangles(id).is_some()
    }

    /// Why a document cannot upload `values` under `id`, or `None` (#311):
    /// the id is free, or holds what the upload replaces — a resident
    /// column for a column, a record for a record, a ragged column for a
    /// ragged column.
    fn document_id_conflict(
        &self,
        id: &str,
        values: &renderer::UploadValues<'_>,
    ) -> Option<String> {
        use renderer::{LogicalColumn, UploadValues};
        let pool = self.renderer.pool();
        let held = if let Some(LogicalColumn::Streamed(_)) = self.renderer.logical_column(id) {
            "a streamed column"
        } else if pool.record(id).is_some() {
            "a record"
        } else if pool.ragged(id).is_some() {
            "a ragged column"
        } else if pool.triangles(id).is_some() {
            "a triangle index column"
        } else if pool.slot(id).is_some() {
            "a column"
        } else {
            return None;
        };
        let wanted = match values {
            UploadValues::BoxRecord(_) => "a record",
            UploadValues::BoxOutliers(_) => "a ragged column",
            _ => "a column",
        };
        (held != wanted).then(|| {
            format!(
                "the id {id} holds {held}, and the document uploads {wanted} under it; \
                 remove it before opening the document"
            )
        })
    }

    /// Replace the chart with a converted legacy document atomically (#311),
    /// in one renderer transaction
    /// (`Renderer::begin_replace_chart_sources`): validate the candidate
    /// (styles and ids), then upload its columns, records and ragged columns
    /// under the documented ids, fit (`fit` receives the candidate pool, the
    /// candidate config and series), remove the previous document's columns
    /// that the new one does not reuse, and swap the config and series, all
    /// in one commit. Every failure leaves the chart, its columns and its
    /// selection as they were; the error names the stage.
    fn replace_document<'a>(
        &mut self,
        family: &'static str,
        config: renderer::Config,
        series: Vec<SeriesConfig>,
        uploads: &[renderer::SeriesUpload<'a>],
        fit: impl FnOnce(
            &renderer::ColumnPool,
            renderer::Config,
            &[SeriesConfig],
        ) -> Result<renderer::Config, JsValue>,
    ) -> Result<(), JsValue> {
        use renderer::{BatchSource, ColumnSource, UploadValues};
        use wasm_bindgen::JsCast;
        let validate = |error: JsValue| document_stage_error(family, "validate", error);

        // ---- validate: the chart, the styles, then the ids ----
        // A document opens on the general chart; a monotonic document shown
        // instead ends with the commit (#271), so it must still be there.
        if let Some(id) = self.monotonic {
            self.renderer
                .monotonic_config(id)
                .map_err(|e| validate(js_err(e)))?;
        }
        self.renderer
            .preflight_chart_styles(&series)
            .map_err(|e| validate(js_err(e)))?;
        for upload in uploads {
            if let Some(conflict) = self.document_id_conflict(&upload.column, &upload.values) {
                return Err(validate(js_err(conflict)));
            }
        }
        // The previous document's columns the new one does not reuse: every
        // id under a document prefix that the replaced series of the general
        // chart read.
        let mut uploaded = std::collections::HashSet::new();
        uploaded
            .try_reserve(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        uploaded.extend(uploads.iter().map(|upload| upload.column.as_str()));
        let mut stale: Vec<String> = Vec::new();
        for item in self
            .renderer
            .chart_series(self.chart_id)
            .map_err(|e| validate(js_err(e)))?
        {
            let outliers = item
                .render_type
                .box_plot()
                .and_then(|b| b.outliers.as_deref());
            for id in [
                Some(item.x_column.as_str()),
                Some(item.y_column.as_str()),
                outliers,
            ]
            .into_iter()
            .flatten()
            {
                let document_column = DOCUMENT_ID_PREFIXES
                    .iter()
                    .any(|prefix| id.starts_with(prefix));
                if document_column && !uploaded.contains(id) && self.document_id_in_use(id) {
                    stale.try_reserve(1).map_err(|e| validate(js_err(e)))?;
                    stale.push(id.to_owned());
                }
            }
        }
        drop(uploaded);
        stale.sort_unstable();
        stale.dedup();
        let mut removals: Vec<&str> = Vec::new();
        removals
            .try_reserve_exact(stale.len())
            .map_err(|e| validate(js_err(e)))?;
        removals.extend(stale.iter().map(String::as_str));

        // The uploads, borrowed from the document through its adapters.
        let mut adapters = Vec::new();
        adapters
            .try_reserve_exact(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        for upload in uploads {
            adapters.push(match &upload.values {
                UploadValues::BoxRecord(_) => DocumentAdapters::Record(
                    upload
                        .values
                        .box_fields()
                        .ok_or_else(|| validate(js_err("a box plot record upload has fields")))?,
                ),
                UploadValues::BoxOutliers(_) => {
                    let (values, offsets) = upload
                        .values
                        .box_outliers()
                        .ok_or_else(|| validate(js_err("a box plot outlier upload has values")))?;
                    DocumentAdapters::Outliers([values, offsets])
                }
                _ => DocumentAdapters::Column,
            });
        }
        // Each field names itself (#160): the record is paired by name, never
        // by the position of the schema's entries.
        let mut fields: Vec<[(&str, &dyn ColumnSource); 9]> = Vec::new();
        fields
            .try_reserve_exact(
                adapters
                    .iter()
                    .filter(|adapter| matches!(adapter, DocumentAdapters::Record(_)))
                    .count(),
            )
            .map_err(|e| validate(js_err(e)))?;
        for adapter in &adapters {
            if let DocumentAdapters::Record(columns) = adapter {
                fields.push(std::array::from_fn(|k| {
                    (
                        renderer::BoxField::ALL[k].name(),
                        &columns[k] as &dyn ColumnSource,
                    )
                }));
            }
        }
        let mut batch: Vec<(&str, BatchSource<'_>)> = Vec::new();
        batch
            .try_reserve_exact(uploads.len())
            .map_err(|e| validate(js_err(e)))?;
        let mut records = fields.iter();
        for (upload, adapter) in uploads.iter().zip(&adapters) {
            let source = match adapter {
                DocumentAdapters::Column => BatchSource::Column(&upload.values),
                DocumentAdapters::Record(_) => BatchSource::Record(renderer::RecordSource {
                    schema: renderer::BOX_PLOT_RECORD,
                    fields: records
                        .next()
                        .ok_or_else(|| validate(js_err("a box plot record has its fields")))?,
                }),
                DocumentAdapters::Outliers([values, offsets]) => {
                    BatchSource::Ragged(renderer::RaggedSource { values, offsets })
                }
            };
            batch.push((upload.column.as_str(), source));
        }

        // As `set_chart_state` does: the shader set the document needs
        // (polar coordinates for a radar) is admitted before it is installed,
        // so its pipelines compile from the start (#299). The fit changes
        // only ranges and ticks, never what decides the set.
        self.renderer.require_shaders_for(&config);

        // ---- upload, fit and the plan, in one transaction ----
        // The fit's own error, and whether it ran: an error after it is the
        // commit stage's (the series checks, the chart plan, the picker).
        let mut fit_error = None;
        let mut fitted = false;
        let count = series.len();
        let guard =
            self.renderer
                .begin_replace_chart_sources(self.chart_id, &batch, &removals, |pool| {
                    match fit(pool, config, &series) {
                        Ok(config) => {
                            fitted = true;
                            Ok((config, series))
                        }
                        Err(error) => {
                            fit_error = Some(error);
                            // Never reported: the stage error below carries the
                            // fit's own.
                            Err(renderer::FiggyError::Internal {
                                invariant: "the document fit failed",
                            })
                        }
                    }
                });
        let guard = match guard {
            Ok(guard) => guard,
            Err(error) => {
                return Err(match fit_error {
                    Some(error) => document_stage_error(family, "fit", error),
                    None if fitted => document_stage_error(family, "commit", js_err(error)),
                    None => {
                        document_stage_error(family, "upload", column_registration_error(error))
                    }
                });
            }
        };

        // ---- commit: columns, removals, config and series at once ----
        guard.commit();
        // The document wrote its own legend.
        self.legend_auto_managed = false;
        self.color_seq = count.max(self.color_seq);
        // Nothing below fails but for a broken invariant: the monotonic
        // chart was checked before the transaction and only this call
        // removes it, and the hit map reads the chart the commit just
        // wrote. Such a failure is reported as an internal error, not as a
        // stage: the new document is shown by then.
        let internal = |what: &str, error: JsValue| {
            js_err(format!(
                "internal error after the {family} document was committed: {what}: {}",
                error
                    .dyn_ref::<js_sys::Error>()
                    .map(|error| String::from(error.message()))
                    .or_else(|| error.as_string())
                    .unwrap_or_default()
            ))
        };
        // A monotonic document shown instead of the general chart ends: the
        // document replaces what the chart shows (#271).
        if let Some(id) = self.monotonic.take() {
            self.renderer
                .monotonic_chart(id)
                .and_then(|chart| chart.remove())
                .map_err(|error| internal("ending the monotonic chart", js_err(error)))?;
            self.set_selected_annotation(None);
            self.view_dirty = true;
        }
        self.sync_hitmap()
            .map_err(|error| internal("rebuilding the hit map", error))
    }
}
