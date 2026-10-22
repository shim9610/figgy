//! Auto-fit on the renderer: fit commit tokens and their column stamps,
//! series and error-bar extent jobs from the column pool, matrix and vector
//! grid extents, and the fit commit entry points.

use super::*;

/// Invocation identity for an atomic two-axis auto-fit commit.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FitCommitToken {
    pub(super) renderer_identity: u64,
    pub(super) chart_id: ChartId,
    pub(super) axes: FitAxisStamp,
    pub(super) series: Vec<FitSeriesStamp>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitAxisStamp {
    pub(super) top_x: (u8, u64, u64),
    pub(super) bottom_x: (u8, u64, u64),
    pub(super) left_y: (u8, u64, u64),
    pub(super) right_y: (u8, u64, u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitSeriesStamp {
    pub(super) series_id: String,
    /// `None` for a render type whose extent is metadata-only (histogram). The
    /// stamp is still
    /// recorded with its column stamps, so a content change in one of those
    /// columns still invalidates the token — dropping the series from the token
    /// entirely would have quietly stopped detecting that.
    pub(super) mode: Option<crate::gpu_errorbar::GpuSeriesFitMode>,
    /// Matrix orientation changes which resolved dimension belongs to x/y even
    /// when the same columns and lattice mode remain selected.
    pub(super) field_orientation: Option<u8>,
    pub(super) columns: Vec<FitColumnStamp>,
}

/// The role slots of a series' fit stamps (`FitColumnStamp::role`): which
/// declared column a stamp describes, so a token can tell a replaced x column
/// from a replaced error column. Fixed roles come first; a matrix's
/// constituent columns take `FIT_ROLE_MATRIX_BASE + index`. A series never
/// carries both vector components and a matrix under the same role space:
/// the points form has components, the grid form has matrices.
pub(super) const FIT_ROLE_X: u32 = 0;

pub(super) const FIT_ROLE_Y: u32 = 1;

pub(super) const FIT_ROLE_ERR_X_LOWER: u32 = 2;

pub(super) const FIT_ROLE_ERR_X_UPPER: u32 = 3;

pub(super) const FIT_ROLE_ERR_Y_LOWER: u32 = 4;

pub(super) const FIT_ROLE_ERR_Y_UPPER: u32 = 5;

pub(super) const FIT_ROLE_VECTOR_U: u32 = 6;

pub(super) const FIT_ROLE_VECTOR_V: u32 = 7;

pub(super) const FIT_ROLE_MATRIX_BASE: u32 = 8;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitColumnStamp {
    pub(super) role: u32,
    pub(super) id: ColumnId,
    pub(super) source: FitColumnSourceStamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FitColumnSourceStamp {
    ResidentAllocation(u64),
    StreamedRegistration { revision: u64, fit_epoch: u64 },
}

pub(super) fn axis_fit_stamp(axis: &AxisOptions) -> (u8, u64, u64) {
    let scale = match axis.scale {
        AxisScale::Linear => 0,
        AxisScale::Logarithmic => 1,
    };
    (scale, axis.min.to_bits(), axis.max.to_bits())
}

pub(super) fn push_fit_column_stamp(
    pool: &ColumnPool,
    streaming_sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    columns: &mut Vec<FitColumnStamp>,
    role: u32,
    id: &str,
) -> Result<()> {
    let source = streaming_sources
        .get(id)
        .map(|streamed| FitColumnSourceStamp::StreamedRegistration {
            revision: streamed.column.revision,
            fit_epoch: streamed.fit_epoch,
        })
        .or_else(|| {
            pool.allocation_epoch(id)
                .map(FitColumnSourceStamp::ResidentAllocation)
        })
        .ok_or_else(|| FiggyError::UnknownColumn { id: id.to_string() })?;
    columns.push(FitColumnStamp {
        role,
        id: id.to_string(),
        source,
    });
    Ok(())
}

pub(super) fn push_fit_error_stamps(
    pool: &ColumnPool,
    streaming_sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    columns: &mut Vec<FitColumnStamp>,
    error: &ErrorRef,
    lower_role: u32,
    upper_role: u32,
) -> Result<()> {
    match error {
        ErrorRef::Symmetric { column } => {
            push_fit_column_stamp(pool, streaming_sources, columns, lower_role, column)?;
            push_fit_column_stamp(pool, streaming_sources, columns, upper_role, column)
        }
        ErrorRef::Asymmetric { lower, upper } => {
            push_fit_column_stamp(pool, streaming_sources, columns, lower_role, lower)?;
            push_fit_column_stamp(pool, streaming_sources, columns, upper_role, upper)
        }
    }
}

pub(super) fn build_fit_token_from_state(
    pool: &ColumnPool,
    streaming_sources: &HashMap<ColumnId, RegisteredStreamColumn>,
    renderer_identity: u64,
    chart_id: ChartId,
    state: &ChartRenderState,
) -> Result<FitCommitToken> {
    let mut series = Vec::new();
    series
        .try_reserve(state.series.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "fit series stamps",
            reason: error.to_string(),
        })?;
    for item in &state.series {
        let mut columns = Vec::new();
        columns
            .try_reserve(6)
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "fit role-column stamps",
                reason: error.to_string(),
            })?;
        push_fit_column_stamp(
            pool,
            streaming_sources,
            &mut columns,
            FIT_ROLE_X,
            &item.x_column,
        )?;
        push_fit_column_stamp(
            pool,
            streaming_sources,
            &mut columns,
            FIT_ROLE_Y,
            &item.y_column,
        )?;
        if let Some(error) = extract_err_x(&item.render_type) {
            push_fit_error_stamps(
                pool,
                streaming_sources,
                &mut columns,
                error,
                FIT_ROLE_ERR_X_LOWER,
                FIT_ROLE_ERR_X_UPPER,
            )?;
        }
        if let Some(error) = extract_err_y(&item.render_type) {
            push_fit_error_stamps(
                pool,
                streaming_sources,
                &mut columns,
                error,
                FIT_ROLE_ERR_Y_LOWER,
                FIT_ROLE_ERR_Y_UPPER,
            )?;
        }
        if let Some(matrix) = fit_matrix(&item.render_type) {
            columns.try_reserve(matrix.columns.len()).map_err(|error| {
                FiggyError::StateAllocationFailed {
                    resource: "fit matrix-column stamps",
                    reason: error.to_string(),
                }
            })?;
            for (index, id) in matrix.columns.iter().enumerate() {
                push_fit_column_stamp(
                    pool,
                    streaming_sources,
                    &mut columns,
                    FIT_ROLE_MATRIX_BASE.saturating_add(u32::try_from(index).unwrap_or(u32::MAX)),
                    id,
                )?;
            }
        }
        // A `Data`-scaled point vector's tips depend on its components, so
        // they stamp the token too.
        if let Some((crate::data_config::VectorComponents::Points { u, v }, style)) =
            extract_vector(&item.render_type)
            && matches!(style.scale, crate::data_config::VectorScale::Data { .. })
        {
            push_fit_column_stamp(pool, streaming_sources, &mut columns, FIT_ROLE_VECTOR_U, u)?;
            push_fit_column_stamp(pool, streaming_sources, &mut columns, FIT_ROLE_VECTOR_V, v)?;
        }
        series.push(FitSeriesStamp {
            series_id: item.series_id.clone(),
            mode: crate::gpu_errorbar::GpuSeriesFitMode::from_render_type(&item.render_type),
            field_orientation: fit_matrix(&item.render_type).map(|matrix| {
                match matrix.orientation {
                    crate::data_config::MatrixOrientation::ColumnsAreX => 0,
                    crate::data_config::MatrixOrientation::ColumnsAreY => 1,
                }
            }),
            columns,
        });
    }

    Ok(FitCommitToken {
        renderer_identity,
        chart_id,
        axes: FitAxisStamp {
            top_x: axis_fit_stamp(&state.config.top_x),
            bottom_x: axis_fit_stamp(&state.config.bottom_x),
            left_y: axis_fit_stamp(&state.config.left_y),
            right_y: axis_fit_stamp(&state.config.right_y),
        },
        series,
    })
}

pub(super) fn current_extent_handle(
    pool: &ColumnPool,
    role: &'static str,
    id: &str,
) -> std::result::Result<ColumnHandle, crate::gpu_errorbar::GpuErrorbarError> {
    let handle = pool.handle_for(id).ok_or_else(|| {
        crate::gpu_errorbar::GpuErrorbarError::UnknownColumn {
            role,
            id: id.to_string(),
        }
    })?;
    if !pool.is_valid_handle(&handle) {
        return Err(crate::gpu_errorbar::GpuErrorbarError::StaleHandle {
            role,
            id: id.to_string(),
            generation: handle.generation,
            current: pool.generation(),
        });
    }
    Ok(handle)
}

pub(super) fn prepared_errorbar_extent_engine(
    engine: Option<&crate::gpu_errorbar::GpuErrorbarExtentEngine>,
) -> std::result::Result<
    &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    engine.ok_or(crate::gpu_errorbar::GpuErrorbarError::EngineNotReady)
}

pub(super) fn begin_errorbar_extent_from_pool(
    engine: &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    value_id: &str,
    lower_id: &str,
    upper_id: &str,
) -> std::result::Result<
    crate::gpu_errorbar::GpuErrorbarExtentTicket,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    engine.begin(
        device,
        queue,
        pool.buffer(),
        current_extent_handle(pool, "value", value_id)?,
        current_extent_handle(pool, "lower error", lower_id)?,
        current_extent_handle(pool, "upper error", upper_id)?,
    )
}

pub(super) fn begin_series_extent_from_pool(
    engine: &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    mode: crate::gpu_errorbar::GpuSeriesExtentMode,
    ids: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
) -> std::result::Result<
    crate::gpu_errorbar::GpuSeriesExtentTicket,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    let optional_handle = |role, id: Option<&str>| {
        id.map(|id| current_extent_handle(pool, role, id))
            .transpose()
    };
    engine.begin_series(
        device,
        queue,
        pool.buffer(),
        mode,
        crate::gpu_errorbar::GpuSeriesExtentColumns {
            x: current_extent_handle(pool, "x", ids.x)?,
            y: current_extent_handle(pool, "y", ids.y)?,
            x_lower: optional_handle("x lower error", ids.x_lower)?,
            x_upper: optional_handle("x upper error", ids.x_upper)?,
            y_lower: optional_handle("y lower error", ids.y_lower)?,
            y_upper: optional_handle("y upper error", ids.y_upper)?,
        },
    )
}

pub(super) fn begin_series_fit_extent_from_pool(
    engine: &crate::gpu_errorbar::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    series: &SeriesConfig,
) -> std::result::Result<
    Option<crate::gpu_errorbar::GpuSeriesExtentTicket>,
    crate::gpu_errorbar::GpuErrorbarError,
> {
    use crate::gpu_errorbar::{GpuFieldExtentColumns, GpuSeriesFitMode};

    let Some(mode) = GpuSeriesFitMode::from_render_type(&series.render_type) else {
        // Histogram bounds are still available synchronously from its edge and
        // value-column upload metadata.
        return Ok(None);
    };
    match mode {
        GpuSeriesFitMode::Paired(crate::gpu_errorbar::GpuSeriesExtentMode::VectorTips) => {
            let Some((crate::data_config::VectorComponents::Points { u, v }, style)) =
                extract_vector(&series.render_type)
            else {
                unreachable!("the tips mode comes from a Data-scaled point vector");
            };
            let crate::data_config::VectorScale::Data { factor } = style.scale else {
                unreachable!("the tips mode comes from a Data-scaled point vector");
            };
            engine
                .begin_vector_tips(
                    device,
                    queue,
                    pool.buffer(),
                    [
                        current_extent_handle(pool, "x", &series.x_column)?,
                        current_extent_handle(pool, "y", &series.y_column)?,
                        current_extent_handle(pool, "vector u", u)?,
                        current_extent_handle(pool, "vector v", v)?,
                    ],
                    factor as f32,
                )
                .map(Some)
        }
        GpuSeriesFitMode::Paired(mode) => {
            fn ids(errors: Option<&ErrorRef>) -> (Option<&str>, Option<&str>) {
                match errors {
                    None => (None, None),
                    Some(ErrorRef::Symmetric { column }) => {
                        (Some(column.as_str()), Some(column.as_str()))
                    }
                    Some(ErrorRef::Asymmetric { lower, upper }) => {
                        (Some(lower.as_str()), Some(upper.as_str()))
                    }
                }
            }
            let (x_lower, x_upper) = ids(extract_err_x(&series.render_type));
            let (y_lower, y_upper) = ids(extract_err_y(&series.render_type));
            begin_series_extent_from_pool(
                engine,
                device,
                queue,
                pool,
                mode,
                crate::gpu_errorbar::GpuSeriesExtentColumnIds {
                    x: &series.x_column,
                    y: &series.y_column,
                    x_lower,
                    x_upper,
                    y_lower,
                    y_upper,
                },
            )
            .map(Some)
        }
        GpuSeriesFitMode::Field(mode) => {
            let matrix = fit_matrix(&series.render_type)
                .expect("field fit mode comes from a matrix render type");
            let x = current_extent_handle(pool, "field x", &series.x_column)?;
            let y = current_extent_handle(pool, "field y", &series.y_column)?;
            let extent = matrix_extent(matrix, x.len_values, y.len_values, |id| {
                pool.handle_for(id).map_or(0, |handle| handle.len_values)
            });
            let (x_cells, y_cells) = match matrix.orientation {
                crate::data_config::MatrixOrientation::ColumnsAreX => (extent.cols, extent.rows),
                crate::data_config::MatrixOrientation::ColumnsAreY => (extent.rows, extent.cols),
            };
            let checked_cells = |role: &'static str, cells: usize| {
                u32::try_from(cells).map_err(|_| {
                    crate::gpu_errorbar::GpuErrorbarError::ValueCountTooLarge { role, len: cells }
                })
            };
            engine
                .begin_field(
                    device,
                    queue,
                    pool.buffer(),
                    mode,
                    GpuFieldExtentColumns {
                        x,
                        y,
                        x_cells: checked_cells("field x cells", x_cells)?,
                        y_cells: checked_cells("field y cells", y_cells)?,
                    },
                )
                .map(Some)
        }
    }
}

/// The grid whose coordinate lattice a series' auto-fit and fit stamps read:
/// a field's matrix, or a grid vector's `u` grid (`v` and a colour grid share
/// its orientation and layout by validation).
pub(super) fn fit_matrix(rt: &DataRenderType) -> Option<&crate::data_config::MatrixRef> {
    extract_matrix(rt).or_else(|| {
        extract_vector(rt).and_then(|(components, _)| components.grid().map(|(u, _)| u))
    })
}

/// The extent a grid vector series draws: the smallest common extent of its
/// `u`, `v` and colour grids, each resolved like a field's matrix.
pub(super) fn vector_grid_extent(
    u: &crate::data_config::MatrixRef,
    v: &crate::data_config::MatrixRef,
    style: &crate::data_config::VectorStyle,
    x_len: usize,
    y_len: usize,
    mut column_len: impl FnMut(&str) -> usize,
) -> MatrixExtent {
    let mut extent = matrix_extent(u, x_len, y_len, &mut column_len);
    let mut narrow = |other: &crate::data_config::MatrixRef| {
        let e = matrix_extent(other, x_len, y_len, &mut column_len);
        extent.truncated |= e.truncated || e.cols != extent.cols || e.rows != extent.rows;
        extent.cols = extent.cols.min(e.cols);
        extent.rows = extent.rows.min(e.rows);
    };
    narrow(v);
    if let Some(crate::data_config::VectorColorMap::Grid(matrix)) = style.color_map.as_ref() {
        narrow(matrix);
    }
    extent
}

/// What a matrix declaration resolves to against the pool's current column
/// lengths.
///
/// The declaration is SSoT; the lengths are facts that arrive with the data. A
/// mismatch is never an error — the smallest common extent is drawn and the
/// surplus is reported — so both halves come out of one place and
/// `series_draw_info_from_pool` and the draw path cannot disagree about the
/// extent they respectively report and paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct MatrixExtent {
    pub(super) cols: usize,
    pub(super) rows: usize,
    pub(super) truncated: bool,
}

/// Resolve a grid declaration against the pool.
///
/// `column_len` reports a constituent column's length, or 0 for one the pool
/// does not hold. A missing column contributes 0 and collapses the grid rather
/// than raising — registration is `validate_renderer_series`' job, and by the
/// time a series is drawn it has already passed.
pub(super) fn matrix_extent(
    matrix: &crate::data_config::MatrixRef,
    x_len: usize,
    y_len: usize,
    mut column_len: impl FnMut(&str) -> usize,
) -> MatrixExtent {
    let (coord_along, coord_across) = matrix.coordinate_cells(x_len, y_len);
    // Columns past what the coordinates can bound are surplus, and their
    // lengths must not shrink the rows the drawn ones support.
    let usable = matrix.columns.len().min(coord_along);
    let mut shortest = usize::MAX;
    let mut longest = 0usize;
    for id in &matrix.columns[..usable] {
        let len = column_len(id);
        shortest = shortest.min(len);
        longest = longest.max(len);
    }
    if usable == 0 {
        shortest = 0;
    }
    let (cols, rows) = matrix.effective_extent(x_len, y_len, shortest);
    // Truncated means some role offered more than was drawn — declared columns,
    // coordinate cells, or values inside a column.
    let truncated =
        matrix.columns.len() > cols || coord_along > cols || coord_across > rows || longest > rows;
    MatrixExtent {
        cols,
        rows,
        truncated,
    }
}

impl Renderer {
    pub fn begin_fit_commit(&self, id: ChartId) -> Result<FitCommitToken> {
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        build_fit_token_from_state(
            &self.pool,
            &self.streaming_sources,
            self.renderer_identity,
            id,
            state,
        )
    }

    /// Request progressive axis fitting for the next automatic streaming
    /// execution. The request is renderer-owned and does not affect explicit
    /// streaming or resident drawing.
    pub fn request_stream_auto_fit(&mut self, id: ChartId, padding: f64) -> Result<()> {
        if !padding.is_finite() || padding < 0.0 {
            return Err(FiggyError::InvalidConfig {
                field: "stream auto-fit padding",
                reason: "padding must be finite and non-negative",
            });
        }
        if self.try_cached_stream_auto_fit(id, padding)? {
            return Ok(());
        }
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        if state.stream_auto_fit_padding == Some(padding) {
            return Ok(());
        }
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.stream_auto_fit_padding = Some(padding);
        state.revisions.desired = desired;
        Ok(())
    }

    pub fn commit_auto_fit_all_if_current(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        if token.renderer_identity != self.renderer_identity
            || !self.chart_states.contains_key(&token.chart_id)
        {
            return Err(FiggyError::StaleStateToken {
                reason: "fit token belongs to another renderer or a removed chart".to_string(),
            });
        }
        let state = self
            .chart_states
            .get(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        let current_fit = build_fit_token_from_state(
            &self.pool,
            &self.streaming_sources,
            self.renderer_identity,
            token.chart_id,
            state,
        )?;
        if current_fit != *token {
            return Err(FiggyError::StaleStateToken {
                reason: "axis range/scale, ordered fit geometry, or source column content changed"
                    .to_string(),
            });
        }

        let state = self
            .chart_states
            .get(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        let desired = state
            .revisions
            .desired
            .successor("chart desired revision")?;
        let config = state.revisions.config.successor("chart config revision")?;
        let view = state.revisions.view.successor("chart view revision")?;
        let raster = state.revisions.raster.successor("chart raster revision")?;
        let visual = self.visual_revision.successor("renderer visual revision")?;

        let state = self
            .chart_states
            .get_mut(&token.chart_id)
            .ok_or(FiggyError::UnknownChart { id: token.chart_id })?;
        crate::chart::apply_auto_fit_all(&mut state.config, x_extent, y_extent, padding);
        // Subplots linked to the parent's axes follow the fitted range.
        state.config.sync_subplot_links(None);
        state.revisions.desired = desired;
        state.revisions.config = config;
        state.revisions.view = view;
        state.revisions.raster = raster;
        self.visual_revision = visual;
        Ok(())
    }

    /// True after explicit fit-bound preparation has published the compute
    /// engine. Construction and ordinary frame preparation leave it uncreated.
    pub fn errorbar_extent_engine_ready(&self) -> bool {
        self.errorbar_extent_engine.is_some()
    }

    /// Prepare and publish the drawable-series fit-bound engine if needed.
    ///
    /// This is the only creation boundary. All hosts must complete it through
    /// exclusive mutable renderer access before shared extent submission. On
    /// wasm it first awaits `GPUDevice.createComputePipelineAsync`; a failed
    /// warmup leaves the engine unpublished.
    pub async fn ensure_errorbar_extent_engine(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_errorbar::GpuErrorbarError> {
        if self.errorbar_extent_engine.is_some() {
            return Ok(());
        }
        let warm_result =
            crate::gpu_errorbar::GpuErrorbarExtentEngine::warm_device_async(&self.device).await;
        self.finish_errorbar_extent_engine_preparation(warm_result)
    }

    pub(super) fn finish_errorbar_extent_engine_preparation(
        &mut self,
        warm_result: std::result::Result<(), crate::gpu_errorbar::GpuErrorbarError>,
    ) -> std::result::Result<(), crate::gpu_errorbar::GpuErrorbarError> {
        warm_result?;
        self.errorbar_extent_engine =
            Some(crate::gpu_errorbar::GpuErrorbarExtentEngine::new_tracked(
                self.device.as_ref(),
                Arc::clone(&self.gpu_ledger),
            ));
        Ok(())
    }

    /// Submit one errorbar-inclusive fit-bound reduction from the current
    /// GPU column-pool mappings. The returned ticket owns its readback and
    /// does not borrow this renderer.
    pub fn begin_errorbar_extent(
        &self,
        value_id: &str,
        lower_id: &str,
        upper_id: &str,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuErrorbarExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        for id in [value_id, lower_id, upper_id] {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        begin_errorbar_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine.as_ref())?,
            &self.device,
            &self.queue,
            &self.pool,
            value_id,
            lower_id,
            upper_id,
        )
    }

    /// Submit one x/y fit-bound reduction over the primitive rows that
    /// the current series mode can draw. The returned ticket owns its
    /// readback and does not borrow this renderer.
    pub fn begin_series_extent(
        &self,
        mode: crate::gpu_errorbar::GpuSeriesExtentMode,
        columns: crate::gpu_errorbar::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_errorbar::GpuSeriesExtentTicket,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        for id in [
            Some(columns.x),
            Some(columns.y),
            columns.x_lower,
            columns.x_upper,
            columns.y_lower,
            columns.y_upper,
        ]
        .into_iter()
        .flatten()
        {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident {
                    id: id.to_owned(),
                });
            }
        }
        begin_series_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine.as_ref())?,
            &self.device,
            &self.queue,
            &self.pool,
            mode,
            columns,
        )
    }

    /// Begin the normalized drawable fit domain for a series. Histograms remain
    /// metadata-only and return `None`; every paired or matrix-backed primitive
    /// returns an owned GPU ticket.
    pub fn begin_series_fit_extent(
        &self,
        series: &SeriesConfig,
    ) -> std::result::Result<
        Option<crate::gpu_errorbar::GpuSeriesExtentTicket>,
        crate::gpu_errorbar::GpuErrorbarError,
    > {
        if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series) {
            return Err(crate::gpu_errorbar::GpuErrorbarError::ColumnNotResident { id });
        }
        begin_series_fit_extent_from_pool(
            prepared_errorbar_extent_engine(self.errorbar_extent_engine.as_ref())?,
            &self.device,
            &self.queue,
            &self.pool,
            series,
        )
    }
}
