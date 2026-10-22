//! Column registration, replacement and removal, and the demo data.

use super::*;

/// A vector series' component columns, its colour-map column or grid,
/// and whether it is the grid form.
pub(super) fn vector_columns(rt: &DataRenderType) -> Option<VectorColumns<'_>> {
    use renderer::data_config::{VectorColorMap, VectorComponents};
    let DataRenderType::Vector { components, style } = rt else {
        return None;
    };
    Some(match components {
        VectorComponents::Points { u, v } => VectorColumns::Points {
            u,
            v,
            color: match style.color_map.as_ref() {
                Some(VectorColorMap::Column(column)) => Some(column),
                _ => None,
            },
        },
        VectorComponents::Grid { u, v } => VectorColumns::Grid {
            u,
            v,
            color: match style.color_map.as_ref() {
                Some(VectorColorMap::Grid(matrix)) => Some(matrix),
                _ => None,
            },
        },
    })
}

pub(super) enum VectorColumns<'a> {
    Points {
        u: &'a str,
        v: &'a str,
        color: Option<&'a str>,
    },
    Grid {
        u: &'a renderer::data_config::MatrixRef,
        v: &'a renderer::data_config::MatrixRef,
        color: Option<&'a renderer::data_config::MatrixRef>,
    },
}

/// Every column id a series' render type references (x/y + error refs).
pub(super) fn referenced_columns(cfg: &SeriesConfig) -> Vec<&str> {
    fn push_ref<'a>(ids: &mut Vec<&'a str>, r: &'a ErrorRef) {
        match r {
            ErrorRef::Symmetric { column } => ids.push(column),
            ErrorRef::Asymmetric { lower, upper } => {
                ids.push(lower);
                ids.push(upper);
            }
        }
    }

    fn push_scatter_ref<'a>(ids: &mut Vec<&'a str>, scatter: &'a renderer::DataScatterStyleConfig) {
        if let Some(column) = &scatter.point_style_index_column {
            ids.push(column);
        }
        if let Some(map) = &scatter.point_color_map {
            ids.push(&map.column);
        }
        if let Some(map) = &scatter.point_size_map {
            ids.push(&map.column);
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
        DataRenderType::Histogram { .. } => {}
        // The grid's constituent columns are referenced columns: a matrix
        // series' derived identity has to change when any of them does, or a
        // replaced grid column would keep serving a stale cache entry.
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => {
            for column in &matrix.columns {
                ids.push(column);
            }
        }
        // The components and the colour value are referenced columns in
        // both forms, like a matrix' constituents.
        DataRenderType::Vector { .. } => match vector_columns(&cfg.render_type) {
            Some(VectorColumns::Points { u, v, color }) => {
                ids.push(u);
                ids.push(v);
                ids.extend(color);
            }
            Some(VectorColumns::Grid { u, v, color }) => {
                for matrix in [Some(u), Some(v), color].into_iter().flatten() {
                    for column in &matrix.columns {
                        ids.push(column);
                    }
                }
            }
            None => {}
        },
    }
    ids
}

pub(super) enum ColumnUploadSource<'a> {
    Scalar(&'a dyn ColumnSource),
    HiLo(&'a dyn HiLoColumnSource),
}

impl FiggyChart {
    pub(super) fn upsert_column_atomic(
        &mut self,
        id: &str,
        len: usize,
        source: ColumnUploadSource<'_>,
    ) -> Result<(), JsValue> {
        // Everything installed after the renderer commit is prepared here,
        // including the checked revision successor and collection capacity.
        let metadata = prepare_column_metadata(
            &self.columns,
            &self.column_revisions,
            self.next_column_revision,
            id,
            len,
        )
        .map_err(js_err)?;
        let ordered_requests =
            active_series_extent_requests(self.chart_series(), &metadata.revisions)
                .map_err(js_err)?;
        let (future_series_extents, pending_series_extents) =
            select_series_extent_job_map(ordered_requests, &self.series_extents, false)
                .map_err(js_err)?;
        let mut ticket_jobs = Vec::new();
        ticket_jobs
            .try_reserve(pending_series_extents.len())
            .map_err(js_err)?;
        let engine_ready = self.renderer.errorbar_extent_engine_ready();

        let guard = match source {
            ColumnUploadSource::Scalar(source) => self.renderer.begin_upsert_column(id, source),
            ColumnUploadSource::HiLo(source) => self.renderer.begin_upsert_hilo_column(id, source),
        }
        .map_err(column_registration_error)?;
        if engine_ready {
            for request in &pending_series_extents {
                let ticket = guard
                    .begin_series_fit_extent(&request.series)
                    .map_err(js_err)?
                    .ok_or_else(|| {
                        js_err("active GPU extent request resolved to a metadata-only series")
                    })?;
                #[cfg(test)]
                record_series_extent_submit();
                let job = future_series_extents
                    .get(&request.key)
                    .cloned()
                    .ok_or_else(|| {
                        js_err("pending series extent key lost its prepared job before commit")
                    })?;
                ticket_jobs.push((ticket, job));
            }
        }
        let prepared_series_extents = PreparedSeriesExtentCache {
            extents: if engine_ready {
                future_series_extents
            } else {
                self.series_extents.clone()
            },
            ticket_jobs,
        };

        guard.commit();
        self.columns = metadata.columns;
        self.column_revisions = metadata.revisions;
        self.next_column_revision = metadata.next_revision;
        self.publish_series_extent_cache(prepared_series_extents);
        Ok(())
    }

    /// Upload a validated batch and install its registry metadata.
    ///
    /// Generic over the adapter so f32 and f64 share one commit without boxing
    /// a source per column — the only per-column allocations on this path are
    /// the two `Vec`s and the metadata maps, all proportional to the *column*
    /// count and independent of `values_per_column`.
    ///
    /// **No series-extent refresh**, for the reason `Renderer::add_columns`
    /// gives for skipping its picker rebuild: every id in a batch is new to the
    /// pool, and `validate_renderer_series` refuses a series that reads a
    /// column the pool does not have — so no existing series can reference one
    /// of these ids, and there is no extent to recompute.
    pub(super) fn commit_column_batch<S: ColumnSource>(
        &mut self,
        ids: &[String],
        len: usize,
        adapters: &[S],
    ) -> Result<(), JsValue> {
        let metadata = prepare_batch_column_metadata(
            &self.columns,
            &self.column_revisions,
            self.next_column_revision,
            ids,
            len,
        )
        .map_err(js_err)?;
        let mut refs: Vec<(&str, &dyn ColumnSource)> = Vec::new();
        refs.try_reserve_exact(ids.len()).map_err(js_err)?;
        refs.extend(
            ids.iter()
                .zip(adapters)
                .map(|(id, adapter)| (id.as_str(), adapter as &dyn ColumnSource)),
        );
        self.renderer
            .add_columns(&refs)
            .map_err(column_registration_error)?;
        self.columns = metadata.columns;
        self.column_revisions = metadata.revisions;
        self.next_column_revision = metadata.next_revision;
        Ok(())
    }
}

#[wasm_bindgen]
impl FiggyChart {
    // ---- column registry (explicit register / update / unregister) ----

    /// Register a new `Float32Array` column. Existing ids are rejected.
    pub fn register_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.columns.contains_key(id),
            ColumnRegistryAction::Register,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF32Column::new(data);
        self.upsert_column_atomic(id, data.len(), ColumnUploadSource::Scalar(&column))
    }

    /// Register a new `Float64Array` column as GPU `(hi, lo)` pairs.
    /// Existing ids are rejected.
    pub fn register_column_f64(&mut self, id: &str, data: &[f64]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.columns.contains_key(id),
            ColumnRegistryAction::Register,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF64Column::new(data);
        self.upsert_column_atomic(id, data.len(), ColumnUploadSource::HiLo(&column))
    }

    /// Register many new columns from **one** flat `Float32Array`.
    ///
    /// `data` holds `ids.len()` columns of `values_per_column` values each, in
    /// id order — the memory layout a matrix already has. This is the wasm
    /// counterpart of `Renderer::add_columns`: one staging buffer and one
    /// submit for the whole batch, so a 5000-column grid costs one upload
    /// rather than 5000 (design B.8).
    ///
    /// Nothing is copied: each column is a borrowed slice of `data` written
    /// straight into the staging buffer.
    ///
    /// All-or-nothing. A rejected batch registers no ids and uploads nothing.
    /// Ragged batches are not supported — a matrix is rectangular by
    /// definition, and unequal lengths still work through
    /// `register_column_f32` one at a time.
    pub fn register_columns_f32(
        &mut self,
        ids: Vec<String>,
        data: &[f32],
        values_per_column: usize,
    ) -> Result<(), JsValue> {
        validate_column_batch(&self.columns, &ids, values_per_column, data.len())
            .map_err(js_err)?;
        let mut adapters = Vec::new();
        adapters.try_reserve_exact(ids.len()).map_err(js_err)?;
        adapters.extend(
            data.chunks_exact(values_per_column)
                .map(BorrowedF32Column::new),
        );
        self.commit_column_batch(&ids, values_per_column, &adapters)
    }

    /// Register many new columns from **one** flat `Float64Array`.
    ///
    /// The f64 counterpart of [`Self::register_columns_f32`], and it keeps the
    /// same precision the single-column `register_column_f64` does: values are
    /// uploaded as the `(hi, lo)` f32 pair, not cast to f32.
    pub fn register_columns_f64(
        &mut self,
        ids: Vec<String>,
        data: &[f64],
        values_per_column: usize,
    ) -> Result<(), JsValue> {
        validate_column_batch(&self.columns, &ids, values_per_column, data.len())
            .map_err(js_err)?;
        let mut adapters = Vec::new();
        adapters.try_reserve_exact(ids.len()).map_err(js_err)?;
        adapters.extend(
            data.chunks_exact(values_per_column)
                .map(BorrowedSplitF64Column::new),
        );
        self.commit_column_batch(&ids, values_per_column, &adapters)
    }

    /// Atomically replace an existing column from a `Float32Array`.
    /// Missing ids are rejected; every accepted call performs an upload.
    pub fn update_register_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.columns.contains_key(id),
            ColumnRegistryAction::Update,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF32Column::new(data);
        self.upsert_column_atomic(id, data.len(), ColumnUploadSource::Scalar(&column))
    }

    /// Atomically replace an existing column from a `Float64Array` as
    /// GPU `(hi, lo)` pairs. Every accepted call performs an upload.
    pub fn update_register_column_f64(&mut self, id: &str, data: &[f64]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.columns.contains_key(id),
            ColumnRegistryAction::Update,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF64Column::new(data);
        self.upsert_column_atomic(id, data.len(), ColumnUploadSource::HiLo(&column))
    }

    /// Unregister a column. Series referencing it are removed too so the
    /// chart can never point at freed data. Auto-managed legends remove
    /// the corresponding rows; freely edited legends preserve their text
    /// and only synchronize recognized series symbols.
    /// Returns `true` when the column existed.
    pub fn remove_column(&mut self, id: &str) -> Result<bool, JsValue> {
        if self.renderer.logical_column(id).is_none() {
            return Ok(false);
        }
        let old_series = self.chart_series().to_vec();
        let mut keep = Vec::new();
        keep.try_reserve(old_series.len()).map_err(js_err)?;
        keep.extend(
            old_series
                .iter()
                .map(|cfg| !referenced_columns(cfg).contains(&id)),
        );
        let mut removed = Vec::new();
        removed.try_reserve(old_series.len()).map_err(js_err)?;
        removed.extend(
            keep.iter()
                .enumerate()
                .filter_map(|(index, keep)| (!keep).then_some(index)),
        );

        let mut next_series = Vec::new();
        next_series.try_reserve(old_series.len()).map_err(js_err)?;
        next_series.extend(
            old_series
                .iter()
                .zip(&keep)
                .filter(|(_, keep)| **keep)
                .map(|(series, _)| series.clone()),
        );
        let mut next_labels = Vec::new();
        next_labels.try_reserve(self.labels.len()).map_err(js_err)?;
        next_labels.extend(
            self.labels
                .iter()
                .zip(&keep)
                .filter(|(_, keep)| **keep)
                .map(|(label, _)| label.clone()),
        );
        let mut next_config = self.chart_config().clone();
        if self.legend_auto_managed {
            for &index in removed.iter().rev() {
                renderer::config::remove_legend_entry(&mut next_config.legend.content, index);
            }
            next_config.legend.visible = !next_config.legend.content.segments.is_empty();
        } else {
            renderer::config::update_legend_symbols_preserving_text(
                &mut next_config.legend.content,
                &next_series,
                next_config.colorbar.as_ref().map(|bar| &bar.colormap),
            );
        }

        let removed_from_renderer = if removed.is_empty() {
            self.renderer.remove_column(id).map_err(js_err)?
        } else {
            self.renderer
                .remove_column_with_chart_config(id, self.chart_id, next_config)
                .map_err(js_err)?
        };
        if !removed_from_renderer {
            return Ok(false);
        }

        self.columns.remove(id);
        self.column_revisions.remove(id);
        debug_assert!(self.renderer.pool().slot(id).is_none());
        let mut keep_iter = keep.iter();
        self.styles.retain(|_| *keep_iter.next().unwrap());
        self.labels = next_labels;
        self.retire_series_extent_cache();
        Ok(true)
    }

    /// Load the bundled demo (sine + RC charge curves) — lets a frontend
    /// see a real chart without wiring data first. Repeated calls replace
    /// the demo registrations and keep the same series declarations.
    pub fn load_demo(&mut self) -> Result<(), JsValue> {
        let (xs, ys) = renderer::demo::sine_data(512);
        let (ts, vs) = renderer::demo::rc_data(512);

        let demo_x = BorrowedCastF32Column::new(&xs);
        let demo_sin = BorrowedCastF32Column::new(&ys);
        let demo_t = BorrowedCastF32Column::new(&ts);
        let demo_rc = BorrowedCastF32Column::new(&vs);
        let metadata = prepare_demo_column_metadata(
            &self.columns,
            &self.column_revisions,
            self.next_column_revision,
            [xs.len(), ys.len(), ts.len(), vs.len()],
        )
        .map_err(js_err)?;
        let declarations = prepare_demo_declarations(
            self.chart_config(),
            self.chart_series(),
            &self.labels,
            self.cycle,
            self.color_seq,
        )
        .map_err(js_err)?;
        for item in &declarations.series {
            for id in referenced_columns(item) {
                if !metadata.columns.contains_key(id) {
                    return Err(js_err(format!(
                        "series '{}' references unregistered column '{id}'",
                        item.series_id
                    )));
                }
            }
        }
        let (scale, _) = self.display_scale_and_panel();
        let mut styles = Vec::new();
        styles
            .try_reserve(declarations.series.len())
            .map_err(js_err)?;
        for item in &declarations.series {
            styles.push(
                self.renderer
                    .create_style_for_series_scaled(item, scale)
                    .map_err(js_err)?,
            );
        }
        let retained_extents = retain_valid_series_extent_jobs(
            &declarations.series,
            &metadata.revisions,
            &self.series_extents,
        )
        .map_err(js_err)?;
        let PreparedDemoDeclarations {
            config,
            series,
            labels,
            color_seq,
        } = declarations;
        let guard = self
            .renderer
            .begin_load_demo(
                self.chart_id,
                &demo_x,
                &demo_sin,
                &demo_t,
                &demo_rc,
                move |pool| {
                    let mut chart = Chart::new(config);
                    chart.auto_fit_x(pool, "demo_x", 0.02)?;
                    chart.auto_fit_y(pool, "demo_sin", 0.10)?;
                    Ok((chart.config().clone(), series))
                },
            )
            .map_err(js_err)?;

        #[cfg(test)]
        if LOAD_DEMO_ABORT_BEFORE_RENDERER_COMMIT.with(Cell::get) {
            return Err(js_err("injected load_demo abort before renderer commit"));
        }

        guard.commit();
        self.columns = metadata.columns;
        self.column_revisions = metadata.revisions;
        self.next_column_revision = metadata.next_revision;
        self.styles = styles;
        self.labels = labels;
        self.color_seq = color_seq;
        self.series_extents = retained_extents;
        Ok(())
    }
}
