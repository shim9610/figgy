//! Column registration, replacement and removal, and the demo data.

use super::*;

/// A vector series' component columns, its colour-map column or grid,
/// and whether it is the grid form.
pub(super) fn vector_columns(rt: &DataRenderType) -> Option<VectorColumns<'_>> {
    use renderer::data_config::{VectorColorMap, VectorComponents};
    let DataRenderType::Vector(VectorSeries { components, style }) = rt else {
        return None;
    };
    Some(match components {
        VectorComponents::Points { u, v, w } => VectorColumns::Points {
            u,
            v,
            w: w.as_deref(),
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
        /// The z component, drawn on a 3D chart only (#211).
        w: Option<&'a str>,
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
    // The model's list (#160): records and ragged columns are resident
    // composites outside the column registry, so they are not visited; a
    // triangle mesh's index column is (#210), and `column_registered`
    // answers for it.
    let mut ids = Vec::new();
    cfg.visit_columns(&mut |id| ids.push(id));
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
        source: ColumnUploadSource<'_>,
    ) -> Result<(), JsValue> {
        let guard = match source {
            ColumnUploadSource::Scalar(source) => self.renderer.begin_upsert_column(id, source),
            ColumnUploadSource::HiLo(source) => self.renderer.begin_upsert_hilo_column(id, source),
        }
        .map_err(column_registration_error)?;
        guard.commit();
        Ok(())
    }

    /// Upload a validated batch.
    ///
    /// Generic over the adapter so f32 and f64 share one commit without boxing
    /// a source per column — the only per-column allocations on this path are
    /// the two `Vec`s, all proportional to the *column*
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
        adapters: &[S],
    ) -> Result<(), JsValue> {
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
        Ok(())
    }
}

/// An open chunked column upload (#311), returned by
/// `FiggyChart.begin_column_upload`. It owns a mapped GPU staging buffer the
/// size of the column; `push` writes typed array chunks straight into it in
/// index order, and `commit(chart)` installs the column at once. Nothing is
/// registered before the commit. `cancel` (or dropping the handle with
/// `free()`) releases the staging buffer. After a commit, a cancel or a
/// commit that failed, the handle is closed and every call but `cancel`
/// throws with `code` `"upload_closed"`.
#[wasm_bindgen]
pub struct ColumnUploadHandle {
    upload: Option<renderer::ColumnUpload>,
    /// The chart that began the upload: only it commits it.
    chart_id: ChartId,
    max_chunk_bytes: u64,
}

impl ColumnUploadHandle {
    fn open(&mut self) -> Result<&mut renderer::ColumnUpload, JsValue> {
        self.upload.as_mut().ok_or_else(|| {
            coded_error(
                "the column upload is closed: it was committed, cancelled or its commit failed",
                "upload_closed",
                [],
            )
        })
    }
}

#[wasm_bindgen]
impl ColumnUploadHandle {
    /// Write one chunk at value `offset`: a `Float32Array` for an `"f32"`
    /// upload, a `Float64Array` for an `"f64"` one, at most
    /// `max_chunk_bytes` bytes. Chunks arrive in order: `offset` must be
    /// `written()`, so a chunk that overlaps written values, leaves a gap
    /// or runs past the length is refused (`code` `"invalid_chunk"`), as is
    /// one of the other array type. A refused chunk changes nothing; the
    /// upload stays open. The array's values are read in place and written
    /// into the mapped staging buffer; nothing is copied on the way.
    pub fn push(&mut self, offset: f64, values: JsValue) -> Result<(), JsValue> {
        let max_chunk_bytes = self.max_chunk_bytes;
        push_chunk(self.open()?, max_chunk_bytes, offset, &values)
    }

    /// Install the column on `chart`, the chart that began the upload, as
    /// one atomic insert or replacement: the series that read it redraw with
    /// the new values. Refused, changing nothing, when a value was not
    /// written (`"invalid_chunk"`), when `chart` is not the chart that began
    /// the upload or the column changed since (`"stale"`), or when the GPU
    /// cannot hold the column (the codes of `register_column_f32`:
    /// `"budget_exceeded"`, `"pool_space"`, `"device_limit"`,
    /// `"allocation_failed"`). The handle is closed either way.
    pub fn commit(&mut self, chart: &mut FiggyChart) -> Result<(), JsValue> {
        self.open()?;
        if chart.chart_id != self.chart_id {
            return Err(coded_error(
                "the column upload was begun on another chart",
                "stale",
                [],
            ));
        }
        let Some(upload) = self.upload.take() else {
            return Err(js_err("internal: an open column upload holds its upload"));
        };
        chart
            .renderer
            .commit_column_upload(upload)
            .map_err(column_upload_error)?;
        Ok(())
    }

    /// Abandon the upload and release its staging buffer. Nothing was
    /// registered. Calling it on a closed handle does nothing.
    pub fn cancel(&mut self) {
        self.upload = None;
    }

    /// Whether the upload is still open.
    pub fn is_open(&self) -> bool {
        self.upload.is_some()
    }

    /// How many values were written: the offset of the next chunk.
    pub fn written(&self) -> f64 {
        self.upload
            .as_ref()
            .map_or(0.0, |upload| upload.written() as f64)
    }

    /// The column's length in values.
    pub fn length(&self) -> f64 {
        self.upload
            .as_ref()
            .map_or(0.0, |upload| upload.len() as f64)
    }
}

/// Write one typed array chunk at `offset` into `upload`'s mapped staging
/// (#311): the checks and codes of `ColumnUploadHandle.push`, which a
/// column transaction's `push` shares (#334).
fn push_chunk(
    upload: &mut renderer::ColumnUpload,
    max_chunk_bytes: u64,
    offset: f64,
    values: &JsValue,
) -> Result<(), JsValue> {
    let offset = js_safe_u64(offset, "offset")?;
    let chunk = JsStreamColumn::from_js(values).map_err(|_| {
        coded_error(
            &format!(
                "column upload {}: a chunk must be a Float32Array or Float64Array",
                upload.id()
            ),
            "invalid_chunk",
            [],
        )
    })?;
    let value_bytes = match chunk.encoding() {
        StreamEncoding::ScalarF32 => 4u64,
        StreamEncoding::HiLoF32 => 8u64,
    };
    let bytes = (chunk.len() as u64).saturating_mul(value_bytes);
    if bytes > max_chunk_bytes {
        return Err(coded_error(
            &format!(
                "column upload {}: the chunk holds {bytes} bytes, above max_chunk_bytes {max_chunk_bytes}",
                upload.id()
            ),
            "invalid_chunk",
            [],
        ));
    }
    upload
        .write_range(offset, chunk.source())
        .map_err(column_upload_error)
}

/// A chunked upload's error with its `code`: `"invalid_chunk"` for a
/// refused chunk or an incomplete commit, `"stale"` for a stale revision or
/// chart, and the column registration codes otherwise.
fn column_upload_error(error: renderer::FiggyError) -> JsValue {
    match &error {
        renderer::FiggyError::InvalidColumnUpload { .. } => {
            coded_error(&error.to_string(), "invalid_chunk", [])
        }
        renderer::FiggyError::StaleStateToken { .. } => {
            coded_error(&error.to_string(), "stale", [])
        }
        _ => column_registration_error(error),
    }
}

/// `max_chunk_bytes` as a positive byte count.
fn positive_chunk_bytes(max_chunk_bytes: f64) -> Result<u64, JsValue> {
    let max_chunk_bytes = js_safe_u64(max_chunk_bytes, "max_chunk_bytes")?;
    if max_chunk_bytes == 0 {
        return Err(js_err("max_chunk_bytes must be positive"));
    }
    Ok(max_chunk_bytes)
}

impl FiggyChart {
    /// Begin the chunked upload of column `id` (#311): the arguments of
    /// `begin_column_upload`, read and handed to the renderer. A column
    /// transaction's `stage` begins its uploads here too (#334).
    fn begin_upload(
        &self,
        id: &str,
        length: f64,
        encoding: &str,
        source_revision: Option<f64>,
    ) -> Result<renderer::ColumnUpload, JsValue> {
        let encoding = match encoding {
            "f32" => StreamEncoding::ScalarF32,
            "f64" => StreamEncoding::HiLoF32,
            other => {
                return Err(js_err(format!(
                    "column upload {id}: encoding {other:?} is not \"f32\" or \"f64\""
                )));
            }
        };
        let length = usize::try_from(js_safe_u64(length, "length")?)
            .map_err(|_| js_err(format!("column upload {id}: length is too large")))?;
        let expected_revision = source_revision
            .map(|revision| js_safe_u64(revision, "source_revision"))
            .transpose()?;
        self.renderer
            .begin_column_upload(id, length, encoding, expected_revision)
            .map_err(column_upload_error)
    }
}

#[wasm_bindgen]
impl FiggyChart {
    // ---- chunked column upload (#311) ----

    /// Begin a chunked upload of the resident column `id`: `length` values,
    /// `encoding` `"f32"` (chunks are `Float32Array`s) or `"f64"`
    /// (`Float64Array`s, kept as GPU `(hi, lo)` pairs). The returned handle
    /// takes the chunks (`push`) and installs the column (`commit(chart)`).
    ///
    /// `source_revision` is the revision of the column the upload replaces,
    /// as `column_revision(id)` reports it, or `null` / `undefined` for an id
    /// that is not registered; anything else throws with `code` `"stale"`,
    /// and so does a commit after the column changed. `max_chunk_bytes`
    /// bounds the bytes of each pushed chunk.
    ///
    /// One GPU staging buffer of the column's size (8 bytes per value) is
    /// created here and held until the commit or the cancel; it counts
    /// against the memory budget, and a budget or device that cannot hold
    /// it throws with the codes of `register_column_f32`. A streamed id or a
    /// reserved id is refused.
    ///
    /// JS: `const up = chart.begin_column_upload("x", n, "f64", chart.column_revision("x"), 1 << 20);`
    pub fn begin_column_upload(
        &mut self,
        id: &str,
        length: f64,
        encoding: &str,
        source_revision: Option<f64>,
        max_chunk_bytes: f64,
    ) -> Result<ColumnUploadHandle, JsValue> {
        let max_chunk_bytes = positive_chunk_bytes(max_chunk_bytes)?;
        let upload = self.begin_upload(id, length, encoding, source_revision)?;
        Ok(ColumnUploadHandle {
            upload: Some(upload),
            chart_id: self.chart_id,
            max_chunk_bytes,
        })
    }

    /// The revision of column `id`: what `begin_column_upload` takes as
    /// `source_revision`. Every upload of a resident column renews it.
    /// `undefined` for an id that is not registered.
    pub fn column_revision(&self, id: &str) -> Option<f64> {
        self.renderer
            .column_revision(id)
            .map(|revision| revision as f64)
    }
}

#[wasm_bindgen]
impl FiggyChart {
    // ---- column registry (explicit register / update / unregister) ----

    /// Register a new `Float32Array` column. Existing ids are rejected.
    pub fn register_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.renderer.logical_column(id).is_some(),
            ColumnRegistryAction::Register,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF32Column::new(data);
        self.upsert_column_atomic(id, ColumnUploadSource::Scalar(&column))
    }

    /// Register a new `Float64Array` column as GPU `(hi, lo)` pairs.
    /// Existing ids are rejected.
    pub fn register_column_f64(&mut self, id: &str, data: &[f64]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.renderer.logical_column(id).is_some(),
            ColumnRegistryAction::Register,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF64Column::new(data);
        self.upsert_column_atomic(id, ColumnUploadSource::HiLo(&column))
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
        validate_column_batch(
            |id| self.renderer.logical_column(id).is_some(),
            &ids,
            values_per_column,
            data.len(),
        )
        .map_err(js_err)?;
        let mut adapters = Vec::new();
        adapters.try_reserve_exact(ids.len()).map_err(js_err)?;
        adapters.extend(
            data.chunks_exact(values_per_column)
                .map(BorrowedF32Column::new),
        );
        self.commit_column_batch(&ids, &adapters)
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
        validate_column_batch(
            |id| self.renderer.logical_column(id).is_some(),
            &ids,
            values_per_column,
            data.len(),
        )
        .map_err(js_err)?;
        let mut adapters = Vec::new();
        adapters.try_reserve_exact(ids.len()).map_err(js_err)?;
        adapters.extend(
            data.chunks_exact(values_per_column)
                .map(BorrowedF64Column::new),
        );
        self.commit_column_batch(&ids, &adapters)
    }

    /// Atomically replace an existing column from a `Float32Array`.
    /// Missing ids are rejected; every accepted call performs an upload.
    pub fn update_register_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.renderer.logical_column(id).is_some(),
            ColumnRegistryAction::Update,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF32Column::new(data);
        self.upsert_column_atomic(id, ColumnUploadSource::Scalar(&column))
    }

    /// Atomically replace an existing column from a `Float64Array` as
    /// GPU `(hi, lo)` pairs. Every accepted call performs an upload.
    pub fn update_register_column_f64(&mut self, id: &str, data: &[f64]) -> Result<(), JsValue> {
        validate_column_registry_action(
            self.renderer.logical_column(id).is_some(),
            ColumnRegistryAction::Update,
        )
        .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        validate_column_data_len(data.len())
            .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
        let column = BorrowedF64Column::new(data);
        self.upsert_column_atomic(id, ColumnUploadSource::HiLo(&column))
    }

    /// Register or replace the box plot record `id` (the schema
    /// `renderer::BOX_PLOT_RECORD`).
    ///
    /// `fields` is an object whose keys are field names and whose values are
    /// `Float32Array`s or `Float64Array`s of one length, one row per box:
    /// `median`, `q1`, `q3`, `whisker_low` and `whisker_high` are required;
    /// `mean`, `sample_count`, `ci_low` and `ci_high` are optional and read as
    /// NaN when absent. A missing box is a row of NaN. The arrays are read in
    /// place and written straight into one batch upload; nothing is copied.
    /// An unknown field, a missing required field or unequal lengths are
    /// errors, and nothing is registered. Records are resident only: a
    /// streamed id, or an id that is a column or a ragged column, is
    /// rejected.
    pub fn register_record(&mut self, id: &str, fields: JsValue) -> Result<(), JsValue> {
        use renderer::{BOX_PLOT_RECORD, MAX_RECORD_FIELDS};
        use wasm_bindgen::JsCast;

        if self.renderer.logical_column(id).is_some() {
            return Err(js_err(format!(
                "record '{id}': the id is a registered column"
            )));
        }
        let object: &js_sys::Object = fields.dyn_ref().ok_or_else(|| {
            js_err(format!(
                "record '{id}': fields must be an object of typed arrays keyed by field name"
            ))
        })?;
        for key in js_sys::Object::keys(object).iter() {
            let name = key.as_string().unwrap_or_default();
            if !BOX_PLOT_RECORD.iter().any(|(field, _)| *field == name) {
                return Err(js_err(format!(
                    "record '{id}': '{name}' is not a field of the box plot record"
                )));
            }
        }
        let mut columns: [Option<JsStreamColumn>; MAX_RECORD_FIELDS] = Default::default();
        let mut names = [""; MAX_RECORD_FIELDS];
        let mut count = 0usize;
        for &(name, _) in BOX_PLOT_RECORD {
            let value = js_sys::Reflect::get(object, &JsValue::from_str(name))?;
            if value.is_undefined() || value.is_null() {
                continue;
            }
            let column = JsStreamColumn::from_js(&value).map_err(|_| {
                js_err(format!(
                    "record '{id}': field '{name}' must be a Float32Array or Float64Array"
                ))
            })?;
            columns[count] = Some(column);
            names[count] = name;
            count += 1;
        }
        let Some(placeholder) = columns[0].as_ref() else {
            return Err(js_err(format!(
                "record '{id}': a record needs at least one field"
            )));
        };
        let placeholder = placeholder.column_source();
        let sources: [(&str, &dyn ColumnSource); MAX_RECORD_FIELDS] = std::array::from_fn(|i| {
            (
                names[i],
                columns[i]
                    .as_ref()
                    .map_or(placeholder, JsStreamColumn::column_source),
            )
        });
        let source = renderer::RecordSource {
            schema: BOX_PLOT_RECORD,
            fields: &sources[..count],
        };
        self.renderer
            .upsert_record(id, &source)
            .map_err(column_registration_error)?;
        Ok(())
    }

    /// Register or replace the ragged column `id`: `offsets.len() - 1`
    /// groups, group `g` being `values[offsets[g]..offsets[g + 1]]`.
    ///
    /// `offsets` starts at 0, never decreases, holds integers and ends at
    /// `values.len()`; anything else is an error and nothing is registered.
    /// Both arrays upload as `(hi, lo)` pairs in one batch, through the same
    /// borrowed adapters as `register_columns_f64`. Resident only, as
    /// `register_record`.
    pub fn register_ragged(
        &mut self,
        id: &str,
        values: &[f64],
        offsets: &[f64],
    ) -> Result<(), JsValue> {
        if self.renderer.logical_column(id).is_some() {
            return Err(js_err(format!(
                "ragged column '{id}': the id is a registered column"
            )));
        }
        let values = BorrowedF64Column::new(values);
        let offsets = BorrowedF64Column::new(offsets);
        let source = renderer::RaggedSource {
            values: &values,
            offsets: &offsets,
        };
        self.renderer
            .upsert_ragged(id, &source)
            .map_err(column_registration_error)?;
        Ok(())
    }

    /// Register or replace the triangle index column `id` (#210): three
    /// vertex indices per triangle, triangle `t` being the vertices
    /// `indices[3t]`, `indices[3t + 1]` and `indices[3t + 2]`.
    ///
    /// `indices` is a `Uint32Array` whose length is a non-zero multiple of 3;
    /// anything else is an error and nothing is registered. wasm-bindgen
    /// copies the JS array into wasm memory for the call; that call-scoped
    /// copy is uploaded and dropped when the call returns, and every index up
    /// to `u32::MAX` reaches the GPU exactly. Resident only, as
    /// `register_record`.
    pub fn register_triangles(&mut self, id: &str, indices: &[u32]) -> Result<(), JsValue> {
        if self.renderer.logical_column(id).is_some() {
            return Err(js_err(format!(
                "triangle index column '{id}': the id is a registered column"
            )));
        }
        let indices = BorrowedU32Column::new(indices);
        let source = renderer::TriangleSource { indices: &indices };
        self.renderer
            .upsert_triangles(id, &source)
            .map_err(column_registration_error)?;
        Ok(())
    }

    /// Unregister the triangle index column `id` and free its GPU memory.
    /// Returns `true` when it existed.
    pub fn remove_triangles(&mut self, id: &str) -> Result<bool, JsValue> {
        self.renderer
            .remove_triangles(id)
            .map_err(column_registration_error)
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
        let old_series = self.chart_series()?.to_vec();
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
        let mut next_config = self.chart_config()?.clone();
        if self.legend_auto_managed {
            let content = &mut next_config.legend.content;
            // The entries of the removed series: the series a symbol names,
            // or, for a symbol that names none, the series at its position.
            for index in (0..renderer::config::legend_entry_count(content)).rev() {
                let gone = match renderer::config::legend_entry_series_id(content, index) {
                    Some(series_id) => old_series
                        .iter()
                        .zip(&keep)
                        .any(|(cfg, keep)| !keep && cfg.series_id == series_id),
                    None => keep.get(index).is_some_and(|keep| !keep),
                };
                if gone {
                    renderer::config::remove_legend_entry(content, index);
                }
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

        if self.renderer.pool().slot(id).is_some() {
            return Err(js_err(
                "internal: the renderer reported a removed column that is still in the pool",
            ));
        }
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
        let declarations = prepare_demo_declarations(
            self.chart_config()?,
            self.chart_series()?,
            self.cycle,
            self.color_seq,
        )
        .map_err(js_err)?;
        for item in &declarations.series {
            for id in referenced_columns(item) {
                // Every column a demo series reads is either one of the
                // demo's own or already registered.
                if !DEMO_COLUMN_IDS.contains(&id) && !self.column_registered(id) {
                    return Err(js_err(format!(
                        "series '{}' references unregistered column '{id}'",
                        item.series_id
                    )));
                }
            }
        }
        self.renderer
            .preflight_chart_styles(&declarations.series)
            .map_err(js_err)?;
        let PreparedDemoDeclarations {
            config,
            series,
            color_seq,
        } = declarations;
        let guard = self
            .renderer
            .begin_replace_chart_columns(
                self.chart_id,
                [
                    (DEMO_COLUMN_IDS[0], &demo_x as &dyn ColumnSource),
                    (DEMO_COLUMN_IDS[1], &demo_sin),
                    (DEMO_COLUMN_IDS[2], &demo_t),
                    (DEMO_COLUMN_IDS[3], &demo_rc),
                ],
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
        self.color_seq = color_seq;
        Ok(())
    }
}

// ---- several chunked uploads and the chart state in one commit (#334) ----

/// An open column transaction (#334), returned by
/// `FiggyChart.begin_columns_transaction`: several chunked uploads, each
/// with its own mapped GPU staging buffer, and optionally the chart's next
/// Config and Series, installed by `commit(chart)` together or not at all.
///
/// - `stage(chart, id, length, encoding, source_revision)` begins one
///   column's upload, as `begin_column_upload` begins one, on the chart that
///   began the transaction. An id staged twice is refused.
/// - `push(id, offset, values)` writes a chunk into that column's staging,
///   in index order, with the checks and codes of `ColumnUploadHandle.push`.
/// - `set_state(config_json, series_json?)` stages the next Config and,
///   when given, the next Series, which may read the staged columns.
/// - `commit(chart)` installs every column, the state and the picker in one
///   renderer transaction; `cancel()` releases every staging buffer.
///
/// A refused `stage`, `push` or `set_state` changes nothing and leaves the
/// transaction open. After a commit, a cancel or a commit that failed, the
/// transaction is closed and every call but `cancel` throws with `code`
/// `"upload_closed"`. Every error carries `stage` (`"stage"`, `"push"`,
/// `"state"` or `"commit"`), `column` (the id it concerns, or `null`) and,
/// for a refusal with one, `code`; a memory refusal carries `requested` and
/// `limit` bytes too. The number of staged columns is not limited.
#[wasm_bindgen]
pub struct ColumnTransaction {
    /// The staged uploads in staging order: the transaction's one list.
    uploads: Vec<renderer::ColumnUpload>,
    /// The candidate Config and, when given, the candidate Series.
    state: Option<(renderer::Config, Option<Vec<SeriesConfig>>)>,
    /// The chart that began the transaction: only it stages and commits.
    chart_id: ChartId,
    max_chunk_bytes: u64,
    open: bool,
}

/// `error` with the column transaction step it failed at (#334): the
/// message names the step and the column, and the `Error` carries `stage`,
/// `column` (`null` when the refusal names none) and the `code` the error
/// had; a registration error's `requestedBytes` and `limitBytes` are
/// repeated as `requested` and `limit`.
fn transaction_error(stage: &str, column: Option<&str>, error: JsValue) -> JsValue {
    use wasm_bindgen::JsCast;
    let cause = match error.dyn_ref::<js_sys::Error>() {
        Some(error) => String::from(error.message()),
        None => error
            .as_string()
            .unwrap_or_else(|| "an error that is not a string".to_owned()),
    };
    let message = match column {
        Some(column) => format!("column transaction {stage} failed for column {column}: {cause}"),
        None => format!("column transaction {stage} failed: {cause}"),
    };
    let object = match error.dyn_into::<js_sys::Error>() {
        Ok(object) => {
            object.set_message(&message);
            object
        }
        Err(_) => js_sys::Error::new(&message),
    };
    let target: &JsValue = object.as_ref();
    let get = |key: &str| {
        js_sys::Reflect::get(target, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
    };
    let (requested, limit) = (get("requestedBytes"), get("limitBytes"));
    let column = column.map_or(JsValue::NULL, JsValue::from_str);
    let fields = [
        ("stage", JsValue::from_str(stage)),
        ("column", column),
        ("requested", requested),
        ("limit", limit),
    ];
    for (key, value) in fields {
        if value.is_undefined() {
            continue;
        }
        match js_sys::Reflect::set(target, &JsValue::from_str(key), &value) {
            Ok(true) => {}
            // The message names the step and the column either way.
            Ok(false) | Err(_) => return JsValue::from_str(&message),
        }
    }
    object.into()
}

impl ColumnTransaction {
    fn check_open(&self, stage: &str, column: Option<&str>) -> Result<(), JsValue> {
        if self.open {
            return Ok(());
        }
        Err(transaction_error(
            stage,
            column,
            coded_error(
                "the column transaction is closed: it was committed, cancelled or its commit failed",
                "upload_closed",
                [],
            ),
        ))
    }

    fn check_chart(&self, chart: &FiggyChart, stage: &str) -> Result<(), JsValue> {
        if chart.chart_id == self.chart_id {
            return Ok(());
        }
        Err(transaction_error(
            stage,
            None,
            coded_error(
                "the column transaction was begun on another chart",
                "stale",
                [],
            ),
        ))
    }

    fn staged(&self, id: &str) -> Option<&renderer::ColumnUpload> {
        self.uploads.iter().find(|upload| upload.id() == id)
    }
}

#[wasm_bindgen]
impl ColumnTransaction {
    /// Stage column `id` on `chart`, the chart that began the transaction:
    /// `length` values of `encoding` (`"f32"` or `"f64"`), replacing the
    /// column at `source_revision` (`column_revision(id)`, or `null` /
    /// `undefined` for an id that is not registered). One GPU staging
    /// buffer of the column's size is created and held until the commit or
    /// the cancel, as `begin_column_upload` creates one; the codes are its
    /// codes (`"stale"`, `"budget_exceeded"`, `"device_limit"`, …). An id
    /// already staged is refused (`"registration_failed"`), as are a
    /// streamed and a reserved id. The `<figgy-chart>` element passes its
    /// chart itself: there it is `stage(id, length, encoding, revision)`.
    pub fn stage(
        &mut self,
        chart: &FiggyChart,
        id: &str,
        length: f64,
        encoding: &str,
        source_revision: Option<f64>,
    ) -> Result<(), JsValue> {
        let fail = |error| transaction_error("stage", Some(id), error);
        self.check_open("stage", Some(id))?;
        self.check_chart(chart, "stage")?;
        if self.staged(id).is_some() {
            return Err(fail(column_registration_error(renderer::FiggyError::Pool(
                AllocError::DuplicateId(id.to_owned()),
            ))));
        }
        self.uploads
            .try_reserve(1)
            .map_err(|error| fail(js_err(error)))?;
        let upload = chart
            .begin_upload(id, length, encoding, source_revision)
            .map_err(fail)?;
        self.uploads.push(upload);
        Ok(())
    }

    /// Write one chunk of staged column `id` at value `offset`: a
    /// `Float32Array` for an `"f32"` column, a `Float64Array` for an `"f64"`
    /// one, at most `max_chunk_bytes` bytes, starting at `written(id)`.
    /// Refused with `code` `"invalid_chunk"` as `ColumnUploadHandle.push`
    /// refuses a chunk, and for an id that is not staged; a refused chunk
    /// changes nothing and the transaction stays open. The array is read in
    /// place into the mapped staging buffer; nothing is copied on the way.
    pub fn push(&mut self, id: &str, offset: f64, values: JsValue) -> Result<(), JsValue> {
        let fail = |error| transaction_error("push", Some(id), error);
        self.check_open("push", Some(id))?;
        let max_chunk_bytes = self.max_chunk_bytes;
        let Some(upload) = self.uploads.iter_mut().find(|upload| upload.id() == id) else {
            return Err(fail(coded_error(
                &format!("column {id} is not staged in this transaction"),
                "invalid_chunk",
                [],
            )));
        };
        push_chunk(upload, max_chunk_bytes, offset, &values).map_err(fail)
    }

    /// Stage the chart's next Config (`config_json`, `get_config` JSON)
    /// and, when `series_json` is given, its next Series (`get_series`
    /// JSON); without it the chart keeps its series. The commit installs
    /// them as `set_config` followed by `set_series` would, legend
    /// included, after checking them against the staged columns. A second
    /// call replaces the first. JSON that does not parse is refused here
    /// and the staged state stays as it was.
    pub fn set_state(
        &mut self,
        config_json: &str,
        series_json: Option<String>,
    ) -> Result<(), JsValue> {
        let fail = |error| transaction_error("state", None, error);
        self.check_open("state", None)?;
        let config: renderer::Config =
            serde_json::from_str(config_json).map_err(|error| fail(js_err(error)))?;
        let series = series_json
            .map(|json| serde_json::from_str::<Vec<SeriesConfig>>(&json))
            .transpose()
            .map_err(|error| fail(js_err(error)))?;
        self.state = Some((config, series));
        Ok(())
    }

    /// Install every staged column and the staged state on `chart`, the
    /// chart that began the transaction, in one commit: the columns, the
    /// Config and Series and the picker publish together, or nothing
    /// changes — values, revisions, Config, Series, selection and the drawn
    /// pixels stay as they were. Refused when a column is not complete
    /// (`"invalid_chunk"`) or changed since it was staged (`"stale"`, its
    /// `column` named), when the GPU cannot hold the columns
    /// (`"budget_exceeded"`, `"pool_space"`, `"device_limit"`,
    /// `"allocation_failed"`), or when the state, or without one the
    /// chart's own Config and Series, does not hold against the staged
    /// columns (`stage` `"state"`, with the code the setters give). A
    /// monotonic chart takes no state here (`"monotonic_chart"`). The
    /// transaction is closed either way.
    pub fn commit(&mut self, chart: &mut FiggyChart) -> Result<(), JsValue> {
        self.check_open("commit", None)?;
        // Closed whatever the commit does; dropping the uploads releases
        // their staging.
        self.open = false;
        let uploads = std::mem::take(&mut self.uploads);
        let state = self.state.take();
        self.check_chart(chart, "commit")?;
        chart.commit_column_transaction(uploads, state)
    }

    /// Abandon the transaction and release every staging buffer. Nothing
    /// was registered. Calling it on a closed transaction does nothing.
    pub fn cancel(&mut self) {
        self.open = false;
        self.uploads = Vec::new();
        self.state = None;
    }

    /// Whether the transaction is still open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// How many values of staged column `id` were written: the offset of
    /// its next chunk. `undefined` for an id that is not staged.
    pub fn written(&self, id: &str) -> Option<f64> {
        self.staged(id).map(|upload| upload.written() as f64)
    }

    /// The length in values of staged column `id`; `undefined` for an id
    /// that is not staged.
    pub fn length(&self, id: &str) -> Option<f64> {
        self.staged(id).map(|upload| upload.len() as f64)
    }

    /// How many columns are staged.
    pub fn staged_count(&self) -> f64 {
        self.uploads.len() as f64
    }

    /// Whether a state was staged with `set_state`.
    pub fn has_state(&self) -> bool {
        self.state.is_some()
    }
}

impl FiggyChart {
    /// The commit of a column transaction (#334): name the column a
    /// refused upload is, prepare the state as `set_config` and
    /// `set_series` would, then install the uploads and the state in one
    /// renderer transaction (`Renderer::commit_column_uploads`).
    fn commit_column_transaction(
        &mut self,
        uploads: Vec<renderer::ColumnUpload>,
        state: Option<(renderer::Config, Option<Vec<SeriesConfig>>)>,
    ) -> Result<(), JsValue> {
        // Every upload as the renderer checks it at commit, one at a time,
        // so the refusal names its column; the renderer checks them again.
        for upload in &uploads {
            self.renderer.check_column_upload(upload).map_err(|error| {
                transaction_error("commit", Some(upload.id()), column_upload_error(error))
            })?;
        }
        let state_error = |error| transaction_error("state", None, error);
        let mut legend_written = false;
        let mut new_series_len = None;
        let state = match state {
            None => None,
            Some(_) if self.monotonic.is_some() => {
                return Err(state_error(coded_error(
                    "a monotonic chart takes its config and series through its own calls; \
                     commit the columns without a state",
                    "monotonic_chart",
                    [],
                )));
            }
            Some((mut config, series)) => {
                // As `set_config` with the chart's series, then, for new
                // series, as `set_series`.
                let current = self.chart_series().map_err(state_error)?;
                legend_written = self
                    .follow_config_legend(&mut config, current)
                    .map_err(state_error)?;
                let mut series = match series {
                    Some(series) => {
                        if self.legend_auto_managed && !legend_written {
                            super::chart::follow_series_legend(&mut config, &series);
                        }
                        new_series_len = Some(series.len());
                        series
                    }
                    None => current.to_vec(),
                };
                super::chart::place_feynman_vertices(&mut series).map_err(state_error)?;
                self.renderer
                    .preflight_chart_styles(&series)
                    .map_err(|error| state_error(js_err(error)))?;
                // The shader set the state needs is admitted first, as
                // `set_chart_state` does (#299).
                self.renderer.require_shaders_for(&config);
                Some((config, series))
            }
        };
        self.renderer
            .commit_column_uploads(self.chart_id, uploads, state)
            .map_err(|error| match error {
                renderer::FiggyError::InvalidConfig { .. }
                | renderer::FiggyError::InvalidSeriesConfig { .. }
                | renderer::FiggyError::UnknownColumn { .. }
                | renderer::FiggyError::ColumnNotResident { .. } => state_error(js_err(error)),
                _ => transaction_error("commit", None, column_upload_error(error)),
            })?;
        if legend_written {
            self.legend_auto_managed = false;
        }
        if let Some(len) = new_series_len {
            self.color_seq = len.max(self.color_seq);
        }
        self.sync_hitmap()
    }
}

#[wasm_bindgen]
impl FiggyChart {
    /// Begin a column transaction (#334): several columns uploaded in
    /// chunks and, optionally, the chart's next Config and Series, installed
    /// together by one `commit(chart)` or not at all. `max_chunk_bytes`
    /// bounds the bytes of each pushed chunk. Nothing is allocated until a
    /// column is staged. See `ColumnTransaction`.
    ///
    /// JS:
    /// ```js
    /// const tx = chart.begin_columns_transaction(1 << 20);
    /// tx.stage(chart, "x", n, "f64", chart.column_revision("x") ?? null);
    /// tx.stage(chart, "y", n, "f64", chart.column_revision("y") ?? null);
    /// tx.push("x", 0, xs);
    /// tx.push("y", 0, ys);
    /// tx.set_state(JSON.stringify(config), JSON.stringify(series));
    /// tx.commit(chart);
    /// ```
    pub fn begin_columns_transaction(
        &self,
        max_chunk_bytes: f64,
    ) -> Result<ColumnTransaction, JsValue> {
        let max_chunk_bytes = positive_chunk_bytes(max_chunk_bytes)?;
        Ok(ColumnTransaction {
            uploads: Vec::new(),
            state: None,
            chart_id: self.chart_id,
            max_chunk_bytes,
            open: true,
        })
    }
}
