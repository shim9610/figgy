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

/// The axes a fit reads or writes: the parent's four, each overlay's y axis
/// and each subplot's cell. A range edit on any of them between `begin` and
/// `commit` retires the token, so a concurrent user edit is never overwritten
/// by a fit computed before it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitAxisStamp {
    pub(super) top_x: (u8, u64, u64),
    pub(super) bottom_x: (u8, u64, u64),
    pub(super) left_y: (u8, u64, u64),
    pub(super) right_y: (u8, u64, u64),
    pub(super) overlays: Vec<(u8, u64, u64)>,
    pub(super) subplots: Vec<FitSubplotStamp>,
}

/// One subplot cell as the fit sees it: its axes, its links (which decide
/// whose range a series fits through `Config::series_fit_roles`) and its own
/// overlays' y axes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitSubplotStamp {
    pub(super) top_x: (u8, u64, u64),
    pub(super) bottom_x: (u8, u64, u64),
    pub(super) left_y: (u8, u64, u64),
    pub(super) right_y: (u8, u64, u64),
    pub(super) link_x: Option<model::layer::CellRef>,
    pub(super) link_y: Option<model::layer::CellRef>,
    pub(super) overlays: Vec<(u8, u64, u64)>,
}

/// The parameters of a series' extent that are not column content: what the
/// host folds into the extent besides the columns' values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum FitExtentParams {
    /// The extent is the columns' values alone.
    Columns,
    /// A `Data`-scaled point vector: the tips are `anchor + factor * (u, v)`.
    VectorTips { factor_bits: u64 },
    /// A histogram: the counts' extent includes the baseline, and the
    /// orientation decides which column is edges and which is counts.
    Histogram { baseline_bits: u64, orientation: u8 },
    /// A stem series (#206): the value axis' extent includes the baseline
    /// (y for a vertical stem, x for a horizontal one), before a line
    /// offset member's shift.
    Stem {
        baseline_bits: u64,
        horizontal: bool,
    },
}

impl FitExtentParams {
    /// The baseline a stem series adds to its value axis' extent, as
    /// `(x part, y part)`; empty for every other kind.
    pub(super) fn stem_extents(&self) -> (crate::chart::FitExtent, crate::chart::FitExtent) {
        let Self::Stem {
            baseline_bits,
            horizontal,
        } = self
        else {
            return (
                crate::chart::FitExtent::EMPTY,
                crate::chart::FitExtent::EMPTY,
            );
        };
        let baseline = crate::chart::FitExtent::point(f64::from_bits(*baseline_bits));
        if *horizontal {
            (baseline, crate::chart::FitExtent::EMPTY)
        } else {
            (crate::chart::FitExtent::EMPTY, baseline)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct FitSeriesStamp {
    pub(super) series_id: String,
    /// The layer that owns the series, which with the subplot links decides
    /// which axes it fits (`Config::series_fit_roles`).
    pub(super) layer: model::layer::LayerRef,
    pub(super) params: FitExtentParams,
    /// `None` for a render type whose extent is metadata-only (histogram). The
    /// stamp is still
    /// recorded with its column stamps, so a content change in one of those
    /// columns still invalidates the token — dropping the series from the token
    /// entirely would have quietly stopped detecting that.
    pub(super) mode: Option<crate::gpu_extent::GpuSeriesFitMode>,
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
        if let Some((crate::data_config::VectorComponents::Points { u, v, .. }, style)) =
            extract_vector(&item.render_type)
            && matches!(style.scale, crate::data_config::VectorScale::Data { .. })
        {
            push_fit_column_stamp(pool, streaming_sources, &mut columns, FIT_ROLE_VECTOR_U, u)?;
            push_fit_column_stamp(pool, streaming_sources, &mut columns, FIT_ROLE_VECTOR_V, v)?;
        }
        let params = match &item.render_type {
            DataRenderType::Vector {
                components: crate::data_config::VectorComponents::Points { .. },
                style,
            } => match style.scale {
                crate::data_config::VectorScale::Data { factor } => FitExtentParams::VectorTips {
                    factor_bits: factor.to_bits(),
                },
                crate::data_config::VectorScale::Pixels { .. } => FitExtentParams::Columns,
            },
            DataRenderType::Histogram { bar } => FitExtentParams::Histogram {
                baseline_bits: bar.baseline.to_bits(),
                orientation: match bar.orientation {
                    crate::data_config::BarOrientation::Vertical => 0,
                    crate::data_config::BarOrientation::Horizontal => 1,
                },
            },
            DataRenderType::Xy(crate::data_config::XySeries {
                stem: Some(stem), ..
            }) => FitExtentParams::Stem {
                baseline_bits: stem.baseline.to_bits(),
                horizontal: stem.direction == crate::data_config::CategoryBarDirection::Horizontal,
            },
            _ => FitExtentParams::Columns,
        };
        series.push(FitSeriesStamp {
            series_id: item.series_id.clone(),
            layer: state.config.layer_of_series(&item.series_id),
            params,
            mode: crate::gpu_extent::GpuSeriesFitMode::from_render_type(&item.render_type),
            field_orientation: fit_matrix(&item.render_type).map(|matrix| {
                match matrix.orientation {
                    crate::data_config::MatrixOrientation::ColumnsAreX => 0,
                    crate::data_config::MatrixOrientation::ColumnsAreY => 1,
                }
            }),
            columns,
        });
    }

    let overlay_stamps = |overlays: &[model::layer::OverlayLayer]| -> Result<Vec<_>> {
        let mut stamps = Vec::new();
        stamps.try_reserve_exact(overlays.len()).map_err(|error| {
            FiggyError::StateAllocationFailed {
                resource: "fit overlay axis stamps",
                reason: error.to_string(),
            }
        })?;
        stamps.extend(
            overlays
                .iter()
                .map(|overlay| axis_fit_stamp(&overlay.y_axis)),
        );
        Ok(stamps)
    };
    let mut subplots = Vec::new();
    subplots
        .try_reserve_exact(state.config.subplots.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "fit subplot stamps",
            reason: error.to_string(),
        })?;
    for subplot in &state.config.subplots {
        subplots.push(FitSubplotStamp {
            top_x: axis_fit_stamp(&subplot.config.top_x),
            bottom_x: axis_fit_stamp(&subplot.config.bottom_x),
            left_y: axis_fit_stamp(&subplot.config.left_y),
            right_y: axis_fit_stamp(&subplot.config.right_y),
            link_x: subplot.link_x,
            link_y: subplot.link_y,
            overlays: overlay_stamps(&subplot.config.overlays)?,
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
            overlays: overlay_stamps(&state.config.overlays)?,
            subplots,
        },
        series,
    })
}

pub(super) fn current_extent_handle(
    pool: &ColumnPool,
    role: &'static str,
    id: &str,
) -> std::result::Result<ColumnHandle, crate::gpu_extent::GpuExtentError> {
    let handle =
        pool.handle_for(id)
            .ok_or_else(|| crate::gpu_extent::GpuExtentError::UnknownColumn {
                role,
                id: id.to_string(),
            })?;
    if !pool.is_valid_handle(&handle) {
        return Err(crate::gpu_extent::GpuExtentError::StaleHandle {
            role,
            id: id.to_string(),
            generation: handle.generation,
            current: pool.generation(),
        });
    }
    Ok(handle)
}

pub(super) fn prepared_errorbar_extent_engine(
    engine: Option<&crate::gpu_extent::GpuErrorbarExtentEngine>,
) -> std::result::Result<
    &crate::gpu_extent::GpuErrorbarExtentEngine,
    crate::gpu_extent::GpuExtentError,
> {
    engine.ok_or(crate::gpu_extent::GpuExtentError::EngineNotReady)
}

pub(super) fn begin_errorbar_extent_from_pool(
    engine: &crate::gpu_extent::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    value_id: &str,
    lower_id: &str,
    upper_id: &str,
) -> std::result::Result<
    crate::gpu_extent::GpuErrorbarExtentTicket,
    crate::gpu_extent::GpuExtentError,
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
    engine: &crate::gpu_extent::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    mode: crate::gpu_extent::GpuSeriesExtentMode,
    ids: crate::gpu_extent::GpuSeriesExtentColumnIds<'_>,
) -> std::result::Result<crate::gpu_extent::GpuSeriesExtentTicket, crate::gpu_extent::GpuExtentError>
{
    let optional_handle = |role, id: Option<&str>| {
        id.map(|id| current_extent_handle(pool, role, id))
            .transpose()
    };
    engine.begin_series(
        device,
        queue,
        pool.buffer(),
        mode,
        crate::gpu_extent::GpuSeriesExtentColumns {
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
    engine: &crate::gpu_extent::GpuErrorbarExtentEngine,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    pool: &ColumnPool,
    series: &SeriesConfig,
) -> std::result::Result<
    Option<crate::gpu_extent::GpuSeriesExtentTicket>,
    crate::gpu_extent::GpuExtentError,
> {
    use crate::gpu_extent::{GpuFieldExtentColumns, GpuSeriesFitMode};

    let Some(mode) = GpuSeriesFitMode::from_render_type(&series.render_type) else {
        // Histogram bounds are still available synchronously from its edge and
        // value-column upload metadata.
        return Ok(None);
    };
    match mode {
        GpuSeriesFitMode::Paired(crate::gpu_extent::GpuSeriesExtentMode::VectorTips) => {
            const TIPS: crate::gpu_extent::GpuExtentError =
                crate::gpu_extent::GpuExtentError::Internal {
                    invariant: "the tips mode comes from a Data-scaled point vector",
                };
            let Some((crate::data_config::VectorComponents::Points { u, v, .. }, style)) =
                extract_vector(&series.render_type)
            else {
                return Err(TIPS);
            };
            let crate::data_config::VectorScale::Data { factor } = style.scale else {
                return Err(TIPS);
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
                crate::gpu_extent::GpuSeriesExtentColumnIds {
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
            let matrix = fit_matrix(&series.render_type).ok_or(
                crate::gpu_extent::GpuExtentError::Internal {
                    invariant: "field fit mode comes from a matrix render type",
                },
            )?;
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
                    crate::gpu_extent::GpuExtentError::ValueCountTooLarge { role, len: cells }
                })
            };
            // A hexagonal grid's coordinates were registered with uniform
            // steps; its hexagons reach past the centres-cells bounds (#134).
            let hexagonal = matches!(
                matrix.grid_layout,
                crate::data_config::GridLayout::Hexagonal
            )
            .then(|| {
                Some((
                    pool.slot(&series.x_column)?.uniform_step?,
                    pool.slot(&series.y_column)?.uniform_step?,
                ))
            })
            .flatten();
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
                .map(|ticket| match hexagonal {
                    Some((x_step, y_step)) => ticket.with_hexagonal_overhang(x_step, y_step),
                    None => ticket,
                })
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
        let result = self.request_stream_auto_fit_unreported(id, padding);
        self.reported(result)
    }

    fn request_stream_auto_fit_unreported(&mut self, id: ChartId, padding: f64) -> Result<()> {
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

    /// Refuse a fit token that no longer describes its chart: another
    /// renderer's, a removed chart's, or one whose ranges, fit geometry or
    /// source columns changed since it was taken.
    fn require_current_fit(&self, token: &FitCommitToken) -> Result<()> {
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
        Ok(())
    }

    pub fn commit_auto_fit_all_if_current(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        let result =
            self.commit_auto_fit_all_if_current_unreported(token, x_extent, y_extent, padding);
        self.reported(result)
    }

    fn commit_auto_fit_all_if_current_unreported(
        &mut self,
        token: &FitCommitToken,
        x_extent: &crate::chart::FitExtent,
        y_extent: &crate::chart::FitExtent,
        padding: f64,
    ) -> Result<()> {
        self.require_current_fit(token)?;

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
    ) -> std::result::Result<(), crate::gpu_extent::GpuExtentError> {
        let result = self.ensure_errorbar_extent_engine_unreported().await;
        self.reported_as(result, crate::gpu_extent::GpuExtentError::from_figgy)
    }

    async fn ensure_errorbar_extent_engine_unreported(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_extent::GpuExtentError> {
        if self.errorbar_extent_engine.is_some() {
            return Ok(());
        }
        let warm_result =
            crate::gpu_extent::GpuErrorbarExtentEngine::warm_device_async(&self.device).await;
        self.finish_errorbar_extent_engine_preparation(warm_result)
    }

    /// [`Self::ensure_errorbar_extent_engine`], without waiting: a native
    /// device compiles the pipelines when they are created.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn ensure_errorbar_extent_engine_blocking(
        &mut self,
    ) -> std::result::Result<(), crate::gpu_extent::GpuExtentError> {
        let result = if self.errorbar_extent_engine.is_some() {
            Ok(())
        } else {
            self.finish_errorbar_extent_engine_preparation(Ok(()))
        };
        self.reported_as(result, crate::gpu_extent::GpuExtentError::from_figgy)
    }

    pub(super) fn finish_errorbar_extent_engine_preparation(
        &mut self,
        warm_result: std::result::Result<(), crate::gpu_extent::GpuExtentError>,
    ) -> std::result::Result<(), crate::gpu_extent::GpuExtentError> {
        warm_result?;
        self.errorbar_extent_engine = Some(
            crate::gpu_extent::GpuErrorbarExtentEngine::new_tracked(
                &self.device,
                Arc::clone(&self.gpu_ledger),
            )
            .with_health(self.health.clone()),
        );
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
        crate::gpu_extent::GpuErrorbarExtentTicket,
        crate::gpu_extent::GpuExtentError,
    > {
        for id in [value_id, lower_id, upper_id] {
            if self.streaming_sources.contains_key(id) {
                return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident {
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
        mode: crate::gpu_extent::GpuSeriesExtentMode,
        columns: crate::gpu_extent::GpuSeriesExtentColumnIds<'_>,
    ) -> std::result::Result<
        crate::gpu_extent::GpuSeriesExtentTicket,
        crate::gpu_extent::GpuExtentError,
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
                return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident {
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
        Option<crate::gpu_extent::GpuSeriesExtentTicket>,
        crate::gpu_extent::GpuExtentError,
    > {
        if let Err(FiggyError::ColumnNotResident { id }) = self.require_resident_series(series) {
            return Err(crate::gpu_extent::GpuExtentError::ColumnNotResident { id });
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

/// What a whole-chart auto-fit does with the active layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoFitAllKind {
    /// A ternary layer: its axes span the triangle, not a data extent.
    Skip,
    /// A parallel chart: each axis fits its columns' statistics (#141).
    Parallel,
    /// Every other layer: the union of its series' drawn extents.
    Extents,
}

/// One series' extent for a whole-chart fit.
pub(super) enum AutoFitSeriesExtent {
    /// A reduction submitted for this fit.
    Pending(crate::gpu_extent::GpuSeriesExtentTicket),
    /// The extent the last committed fit found for the same stamp.
    Known(Option<crate::gpu_extent::GpuSeriesExtent>),
}

pub(super) struct AutoFitSeries {
    pub(super) stamp: FitSeriesStamp,
    pub(super) x: bool,
    pub(super) y: bool,
    pub(super) extent: AutoFitSeriesExtent,
}

/// A whole-chart auto-fit in flight. [`Renderer::begin_auto_fit_all`] takes
/// it; [`Self::resolve`] waits for its reductions without borrowing the
/// renderer; [`Renderer::commit_auto_fit_all`] applies it, unless the chart
/// changed in between.
pub struct AutoFitAllTicket {
    token: FitCommitToken,
    layer: crate::LayerRef,
    padding: f64,
    kind: AutoFitAllKind,
    /// One entry per series the fitted layer draws (#160).
    pub(super) series: Vec<AutoFitSeries>,
}

/// A whole-chart auto-fit whose reductions have completed, ready for
/// [`Renderer::commit_auto_fit_all`].
pub struct ResolvedAutoFitAll {
    token: FitCommitToken,
    layer: crate::LayerRef,
    padding: f64,
    kind: AutoFitAllKind,
    /// One entry per series the fitted layer draws, with its extent.
    series: Vec<(
        FitSeriesStamp,
        bool,
        bool,
        Option<crate::gpu_extent::GpuSeriesExtent>,
    )>,
}

impl AutoFitAllTicket {
    /// Wait for every reduction this fit submitted: the bundle's poll thread
    /// completes the waits on native (see [`crate::RendererDevice::new`]).
    pub async fn resolve(
        mut self,
    ) -> std::result::Result<ResolvedAutoFitAll, crate::gpu_extent::GpuExtentError> {
        for job in &mut self.series {
            if let AutoFitSeriesExtent::Pending(ticket) = &mut job.extent {
                ticket.wait().await?;
            }
        }
        self.finish()
    }

    /// [`Self::resolve`], polling the device on this thread until every
    /// reduction completes.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn resolve_blocking(
        mut self,
    ) -> std::result::Result<ResolvedAutoFitAll, crate::gpu_extent::GpuExtentError> {
        for job in &mut self.series {
            if let AutoFitSeriesExtent::Pending(ticket) = &mut job.extent {
                ticket.wait_blocking()?;
            }
        }
        self.finish()
    }

    /// The fit if every reduction has completed, or the ticket back, without
    /// blocking.
    pub fn try_resolve(
        mut self,
    ) -> std::result::Result<
        crate::TryResolve<ResolvedAutoFitAll, Self>,
        crate::gpu_extent::GpuExtentError,
    > {
        let mut ready = true;
        for job in &mut self.series {
            if let AutoFitSeriesExtent::Pending(ticket) = &mut job.extent {
                // Every one, so the first call hands every wait to the poll
                // thread.
                ready &= ticket.poll_ready()?;
            }
        }
        if !ready {
            return Ok(crate::TryResolve::Pending(self));
        }
        self.finish().map(crate::TryResolve::Ready)
    }

    /// After every wait: the extents, for [`Renderer::commit_auto_fit_all`].
    fn finish(self) -> std::result::Result<ResolvedAutoFitAll, crate::gpu_extent::GpuExtentError> {
        let mut series = Vec::new();
        series.try_reserve(self.series.len()).map_err(|error| {
            crate::gpu_extent::GpuExtentError::from_figgy(FiggyError::StateAllocationFailed {
                resource: "auto-fit series extents",
                reason: error.to_string(),
            })
        })?;
        for job in self.series {
            let extent = match job.extent {
                AutoFitSeriesExtent::Pending(mut ticket) => ticket.finish()?,
                AutoFitSeriesExtent::Known(extent) => extent,
            };
            series.push((job.stamp, job.x, job.y, extent));
        }
        Ok(ResolvedAutoFitAll {
            token: self.token,
            layer: self.layer,
            padding: self.padding,
            kind: self.kind,
            series,
        })
    }
}

fn fit_extent_of(extent: crate::gpu_extent::GpuErrorbarExtent) -> crate::chart::FitExtent {
    crate::chart::FitExtent {
        min: extent.min,
        max: extent.max,
        min_positive: extent.min_positive,
    }
}

/// The columns a histogram's fit reads, as (edges, counts): the edge column
/// lies along the bars' run, the count column along their length.
pub(super) fn histogram_fit_columns<'a>(
    series: &'a SeriesConfig,
    orientation: &crate::data_config::BarOrientation,
) -> (&'a str, &'a str) {
    if matches!(orientation, crate::data_config::BarOrientation::Vertical) {
        (&series.x_column, &series.y_column)
    } else {
        (&series.y_column, &series.x_column)
    }
}

/// The config a layer's axes live in: the chart's, or a subplot cell's.
fn layer_config(config: &Config, layer: crate::LayerRef) -> Option<&Config> {
    match layer {
        crate::LayerRef::Subplot(index) | crate::LayerRef::SubplotOverlay(index, _) => {
            config.subplots.get(index).map(|subplot| &subplot.config)
        }
        _ => Some(config),
    }
}

impl Renderer {
    /// Begin fitting the active layer's axes to the data it draws: a ternary
    /// layer is left as it is, a parallel chart fits each axis to its columns,
    /// and every other layer fits its x and y axes to the union of its series'
    /// drawn extents (histograms from their columns' statistics, everything
    /// else from a GPU reduction, reused when the series' columns are
    /// unchanged since the last fit), and a 3D layer's z axis to its height
    /// columns. The annotation layer fits the base axes.
    ///
    /// The ticket owns its reductions and does not borrow the renderer; await
    /// [`AutoFitAllTicket::resolve`] and pass the result to
    /// [`Self::commit_auto_fit_all`]. A series that needs a reduction requires
    /// the extent engine: await [`Self::ensure_errorbar_extent_engine`] first.
    /// A streamed source is refused with `ColumnNotResident`.
    pub fn begin_auto_fit_all(
        &self,
        id: ChartId,
        padding: f64,
    ) -> std::result::Result<AutoFitAllTicket, crate::gpu_extent::GpuExtentError> {
        let result = self.begin_auto_fit_all_unreported(id, padding);
        self.reported_as(result, crate::gpu_extent::GpuExtentError::from_figgy)
    }

    fn begin_auto_fit_all_unreported(
        &self,
        id: ChartId,
        padding: f64,
    ) -> std::result::Result<AutoFitAllTicket, crate::gpu_extent::GpuExtentError> {
        use crate::gpu_extent::GpuExtentError;
        if !padding.is_finite() || padding < 0.0 {
            return Err(GpuExtentError::from_figgy(FiggyError::InvalidConfig {
                field: "auto-fit padding",
                reason: "padding must be finite and non-negative",
            }));
        }
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })
            .map_err(GpuExtentError::from_figgy)?;
        let layer = match self.active_layer(id).map_err(GpuExtentError::from_figgy)? {
            crate::LayerRef::Annotations => crate::LayerRef::Base,
            layer => layer,
        };
        let token = build_fit_token_from_state(
            &self.pool,
            &self.streaming_sources,
            self.renderer_identity,
            id,
            state,
        )
        .map_err(GpuExtentError::from_figgy)?;
        let kind = if layer_config(&state.config, layer).is_some_and(|c| c.ternary().is_some()) {
            AutoFitAllKind::Skip
        } else if state.config.parallel().is_some() {
            AutoFitAllKind::Parallel
        } else {
            AutoFitAllKind::Extents
        };
        let mut series = Vec::new();
        if kind == AutoFitAllKind::Extents {
            series.try_reserve(state.series.len()).map_err(|error| {
                GpuExtentError::from_figgy(FiggyError::StateAllocationFailed {
                    resource: "auto-fit series extents",
                    reason: error.to_string(),
                })
            })?;
            for (item, stamp) in state.series.iter().zip(&token.series) {
                let (x, y) = state.config.series_fit_roles(layer, &item.series_id);
                // A histogram fits from its columns' statistics at commit.
                if !(x || y) || stamp.mode.is_none() {
                    continue;
                }
                let extent = match state.fit_extents.get(stamp) {
                    Some(known) => AutoFitSeriesExtent::Known(*known),
                    None => match self.begin_series_fit_extent(item)? {
                        Some(ticket) => AutoFitSeriesExtent::Pending(ticket),
                        None => AutoFitSeriesExtent::Known(None),
                    },
                };
                series.push(AutoFitSeries {
                    stamp: stamp.clone(),
                    x,
                    y,
                    extent,
                });
            }
        }
        Ok(AutoFitAllTicket {
            token,
            layer,
            padding,
            kind,
            series,
        })
    }

    /// Apply a resolved whole-chart auto-fit, unless an axis range or scale,
    /// the fit geometry or a source column changed since it began
    /// (`StaleStateToken`). Remembers each series' extent for the next fit.
    pub fn commit_auto_fit_all(&mut self, resolved: ResolvedAutoFitAll) -> Result<()> {
        let result = self.commit_auto_fit_all_unreported(resolved);
        self.reported(result)
    }

    fn commit_auto_fit_all_unreported(&mut self, resolved: ResolvedAutoFitAll) -> Result<()> {
        self.require_current_fit(&resolved.token)?;
        let id = resolved.token.chart_id;
        let state = self
            .chart_states
            .get(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        let padding = resolved.padding;
        let mut chart = crate::chart::Chart::new(state.config.clone());
        match resolved.kind {
            AutoFitAllKind::Skip => return Ok(()),
            AutoFitAllKind::Parallel => {
                chart.auto_fit_parallel_axes(&self.pool, &state.series, padding)?;
            }
            AutoFitAllKind::Extents => {
                let layer = resolved.layer;
                let mut x_extent = crate::chart::FitExtent::EMPTY;
                let mut y_extent = crate::chart::FitExtent::EMPTY;
                for (stamp, x, y, extent) in &resolved.series {
                    let Some(extent) = extent else {
                        continue;
                    };
                    // A stem's baseline joins its value axis (#206), before
                    // a line offset member's shift.
                    let (stem_x, stem_y) = stamp.params.stem_extents();
                    let (mut x_own, mut y_own) = (fit_extent_of(extent.x), fit_extent_of(extent.y));
                    x_own.union(&stem_x);
                    y_own.union(&stem_y);
                    // A line offset member fits where it is drawn.
                    let (x_part, y_part) = crate::chart::Chart::line_offset_fit_extents(
                        &state.config,
                        &stamp.series_id,
                        x_own,
                        y_own,
                    );
                    if *x {
                        x_extent.union(&x_part);
                    }
                    if *y {
                        y_extent.union(&y_part);
                    }
                }
                for item in &state.series {
                    let DataRenderType::Histogram { bar } = &item.render_type else {
                        continue;
                    };
                    let (x, y) = state.config.series_fit_roles(layer, &item.series_id);
                    if !(x || y) {
                        continue;
                    }
                    let (edge_column, count_column) = histogram_fit_columns(item, &bar.orientation);
                    let edge = crate::chart::Chart::slot_extent(&self.pool, edge_column)?;
                    let mut count = crate::chart::Chart::slot_extent(&self.pool, count_column)?;
                    count.union(&crate::chart::FitExtent {
                        min: bar.baseline,
                        max: bar.baseline,
                        min_positive: (bar.baseline > 0.0).then_some(bar.baseline),
                    });
                    let (x_part, y_part) = match &bar.orientation {
                        crate::data_config::BarOrientation::Vertical => (edge, count),
                        crate::data_config::BarOrientation::Horizontal => (count, edge),
                    };
                    if x {
                        x_extent.union(&x_part);
                    }
                    if y {
                        y_extent.union(&y_part);
                    }
                }
                chart.auto_fit_layer_extent(layer, &x_extent, &y_extent, padding);
                if layer_config(chart.config(), layer).is_some_and(|c| c.view_3d().is_some()) {
                    chart.auto_fit_view_3d_z(&self.pool, layer, &state.series, padding)?;
                }
            }
        }
        let mut fit_extents = HashMap::new();
        fit_extents
            .try_reserve(resolved.series.len())
            .map_err(|error| FiggyError::StateAllocationFailed {
                resource: "auto-fit extent cache",
                reason: error.to_string(),
            })?;
        fit_extents.extend(
            resolved
                .series
                .into_iter()
                .map(|(stamp, _, _, extent)| (stamp, extent)),
        );
        self.set_chart_config(id, chart.config().clone())?;
        let state = self
            .chart_states
            .get_mut(&id)
            .ok_or(FiggyError::UnknownChart { id })?;
        state.fit_extents = fit_extents;
        Ok(())
    }
}
