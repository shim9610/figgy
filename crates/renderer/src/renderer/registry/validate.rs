//! Series and chart validation against the registered columns.

use super::*;

/// Validate a series list against the pool and the chart's own `Config`.
///
/// The config is needed because a field series is only fully declared with it:
/// `Config.colorbar` owns the z range and colormap.
pub(in crate::renderer) fn validate_renderer_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    series: &[SeriesConfig],
) -> Result<()> {
    // Every rule on the series list that needs no column pool is the
    // model's (#160): ids, the chart-wide kinds' rules and polar axes.
    config.validate_series(series).map_err(config_error)?;
    // A hexagonal grid (#134) is a flat heatmap on linear axes of a flat
    // chart, and its coordinate columns are uniform: the lattice the shader
    // walks is the one the data describes.
    let hexagonal = |item: &SeriesConfig| {
        extract_matrix(&item.render_type)
            .is_some_and(|m| m.grid_layout == crate::data_config::GridLayout::Hexagonal)
    };
    for item in series.iter().filter(|item| hexagonal(item)) {
        for (role, id) in [("x", &item.x_column), ("y", &item.y_column)] {
            if pool.uniform_step(id).is_none() {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    kind: crate::RuleKind::HexagonalCoordinatesNotUniform {
                        role: role.to_string(),
                        column: id.to_string(),
                    },
                });
            }
        }
    }
    for item in series {
        if let Some(Err(e)) = item.label.as_ref().map(|label| label.validate_math()) {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                kind: crate::RuleKind::InvalidLabel {
                    detail: e.to_string(),
                },
            });
        }
        // A DataLess series registers no column (#203, #234): its values
        // are SSOT, held to the model's rules above.
        if item.is_dataless() {
            continue;
        }
        if let Some(mesh) = item.tri_mesh() {
            validate_tri_mesh_series(pool, config, item, mesh)?;
            continue;
        }
        validate_registered_column(pool, &item.x_column)?;
        if let Some(boxes) = item.render_type.box_plot() {
            // A box plot reads a record and ragged columns, not columns,
            // besides its violin's densities and its raw points.
            let record = (!item.y_column.is_empty()).then_some(item.y_column.as_str());
            for id in record.into_iter().chain(boxes.ragged_columns()) {
                if !pool.contains_composite(id) {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        kind: crate::RuleKind::BoxPlotNotResident { id: id.to_string() },
                    });
                }
            }
            for id in boxes.plain_columns() {
                validate_registered_column(pool, id)?;
            }
        } else {
            validate_registered_column(pool, &item.y_column)?;
        }
        if let Some(bars) = item.render_type.range_bars() {
            for id in bars.extra_columns() {
                validate_registered_column(pool, id)?;
            }
        }
        if let Some(c) = item.render_type.ternary_c_column() {
            validate_registered_column(pool, c)?;
        }
        if let Some(scatter) = extract_scatter(&item.render_type) {
            if let Some(column) = scatter.point_style_index_column.as_deref() {
                validate_registered_column(pool, column)?;
            }
            if let Some(map) = scatter.point_color_map.as_ref() {
                validate_registered_column(pool, &map.column)?;
                if config.colorbar.is_none() {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        kind: crate::RuleKind::ScatterColorMapNeedsColorbar,
                    });
                }
            }
            if let Some(map) = scatter.point_size_map.as_ref() {
                validate_registered_column(pool, &map.column)?;
                map.validate()
                    .map_err(|reason| FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        kind: reason,
                    })?;
            }
        }
        if let Some(errorbar) = extract_errorbar_style(&item.render_type)
            && let Some(column) = errorbar.error_bar_style_index_column.as_deref()
        {
            validate_registered_column(pool, column)?;
        }
        if let Some(error) = extract_err_x(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
        if let Some(error) = extract_err_y(&item.render_type) {
            validate_error_ref_columns(pool, error)?;
        }
        if requires_colorbar(&item.render_type) && config.colorbar.is_none() {
            return Err(FiggyError::InvalidSeriesConfig {
                series_id: item.series_id.clone(),
                kind: crate::RuleKind::FieldNeedsColorbar,
            });
        }
        validate_contour_level_count(&item.series_id, &item.render_type)?;
        validate_contour_label_config(&item.series_id, &item.render_type)?;
        validate_line_fill(pool, series.iter(), item)?;
        // Every axis column of a parallel series is read on the GPU, and
        // its colour column maps through the colourbar (#141).
        if let Some(parallel) = item.render_type.parallel() {
            for column in parallel.columns {
                validate_registered_column(pool, column)?;
            }
            if let Some(column) = parallel.color_column {
                validate_registered_column(pool, column)?;
                if config.colorbar.is_none() {
                    return Err(FiggyError::InvalidSeriesConfig {
                        series_id: item.series_id.clone(),
                        kind: crate::RuleKind::ParallelColorMapNeedsColorbar,
                    });
                }
            }
        }
        if let Some(matrix) = extract_matrix(&item.render_type) {
            // A grid column that is not registered would read whatever else
            // happens to be at that offset. Count mismatches between the
            // declaration and the data are *not* checked here — those are drawn
            // to the smallest common extent and reported, never rejected.
            for column in &matrix.columns {
                validate_registered_column(pool, column)?;
            }
        }
        // An image's other two channels alike (#139), after its own rules.
        if let Some([_, green, blue]) = item.render_type.rgb_image() {
            for column in green.columns.iter().chain(&blue.columns) {
                validate_registered_column(pool, column)?;
            }
        }
        if let Some((components, style)) = extract_vector(&item.render_type) {
            validate_vector_series(pool, config, item, components, style)?;
        }
        validate_streamline_series(pool, config, item)?;
    }
    Ok(())
}

/// A triangle mesh series' renderer rules (#210), after the model's: its
/// coordinate and value columns are registered (on a 3D chart its height
/// column too), its triangles are a resident triangle index column, a
/// colourbar colours it, its contour level count is in range, and every
/// index names a vertex the mesh has.
///
/// The vertex count is the shortest of `x_column`, `y_column`, on a 3D chart
/// `z_column`, and, for vertex values, the value column; only the index
/// column's statistics and the column lengths are read, never an index. A
/// later upload that shortens a column is not re-checked: the draw drops the
/// triangles that name a vertex past the end and `series_draw_info` reports
/// the truncation.
pub(in crate::renderer) fn validate_tri_mesh_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    item: &SeriesConfig,
    mesh: &crate::tri_mesh::TriMeshSeries,
) -> Result<()> {
    let fail = |reason: crate::RuleKind| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        kind: reason,
    };
    validate_registered_column(pool, &item.x_column)?;
    validate_registered_column(pool, &item.y_column)?;
    validate_registered_column(pool, &mesh.values.column)?;
    // The height column bounds the draw where the mesh is drawn in 3D
    // (`Config::drawn_z_column`); on a flat chart it is kept and unused.
    let height = config.drawn_z_column(item);
    if let Some(z) = height {
        validate_registered_column(pool, z)?;
    }
    let stats = pool.triangle_stats(&mesh.triangles).ok_or_else(|| {
        fail(crate::RuleKind::TrianglesNotResident {
            id: mesh.triangles.to_string(),
        })
    })?;
    if config.colorbar.is_none() {
        return Err(fail(crate::RuleKind::TriMeshNeedsColorbar));
    }
    if let Some(contour) = &mesh.contour
        && contour.levels.len() > crate::data_config::MAX_CONTOUR_LEVELS
    {
        return Err(fail(crate::RuleKind::ContourLevelCount {
            count: contour.levels.len() as u64,
            max: crate::data_config::MAX_CONTOUR_LEVELS as u64,
        }));
    }
    let len = |id: &str| pool.column_len(id).unwrap_or(0);
    let faces = mesh.values.at == crate::tri_mesh::MeshValueLocation::Faces;
    let mut shortest = (len(&item.x_column), item.x_column.as_str());
    let values = (!faces).then(|| (len(&mesh.values.column), mesh.values.column.as_str()));
    let heights = height.map(|z| (len(z), z));
    for candidate in [
        Some((len(&item.y_column), item.y_column.as_str())),
        values,
        heights,
    ]
    .into_iter()
    .flatten()
    {
        if candidate.0 < shortest.0 {
            shortest = candidate;
        }
    }
    let (vertices, column) = shortest;
    if u64::from(stats.max_index) >= vertices {
        return Err(fail(crate::RuleKind::TriangleIndexPastVertices {
            index: u64::from(stats.max_index),
            vertices,
            column: column.to_string(),
        }));
    }
    if faces && len(&mesh.values.column) < u64::from(stats.triangles) {
        return Err(fail(crate::RuleKind::FaceValueColumnShort {
            column: mesh.values.column.clone(),
            len: len(&mesh.values.column),
            triangles: u64::from(stats.triangles),
        }));
    }
    Ok(())
}

/// A streamline series' rules: its own (`data_config::validate_streamline_series`:
/// a linear line without a fill and valid arrows), a registered colour
/// column, and the colourbar a colour map maps through.
pub(in crate::renderer) fn validate_streamline_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    item: &SeriesConfig,
) -> Result<()> {
    let Some(streamlines) = item.render_type.streamlines() else {
        return Ok(());
    };
    let invalid = |reason: crate::RuleKind| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        kind: reason,
    };
    crate::data_config::validate_streamline_series(item).map_err(invalid)?;
    if let Some(map) = streamlines.color_map {
        validate_registered_column(pool, &map.column)?;
        if config.colorbar.is_none() {
            return Err(invalid(crate::RuleKind::StreamlineColorMapNeedsColorbar));
        }
    }
    Ok(())
}

/// The polar and ternary coordinate systems of the chart against its own
/// axes.
pub(in crate::renderer) fn validate_polar_config(config: &Config) -> Result<()> {
    config
        .validate_polar()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "polar",
            kind: reason,
        })?;
    config
        .validate_ternary()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "ternary",
            kind: reason,
        })?;
    config
        .validate_view_3d()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "view_3d",
            kind: reason,
        })?;
    config
        .validate_parallel()
        .map_err(|reason| FiggyError::InvalidConfig {
            field: "parallel",
            kind: reason,
        })
}

/// A model rule's refusal as the renderer's error: a series rule names its
/// series; a chart-wide rule that names none reports its field
/// (`InvalidConfig`), as the line offset group always did, or an empty series
/// id for the other kinds' rules.
pub(in crate::renderer) fn config_error(error: crate::ConfigError) -> FiggyError {
    match error.series_id {
        Some(series_id) => FiggyError::InvalidSeriesConfig {
            series_id,
            kind: error.kind,
        },
        None if error.field == "line_offsets" => FiggyError::InvalidConfig {
            field: error.field,
            kind: error.kind,
        },
        None => FiggyError::InvalidSeriesConfig {
            series_id: String::new(),
            kind: error.kind,
        },
    }
}

/// A vector series' own rules: a valid style, registered component columns,
/// grids that agree on orientation and layout, a colour map in the same form
/// as the components, and the colourbar it maps through.
pub(in crate::renderer) fn validate_vector_series(
    pool: &impl ColumnRegistration,
    config: &Config,
    item: &SeriesConfig,
    components: &crate::data_config::VectorComponents,
    style: &crate::data_config::VectorStyle,
) -> Result<()> {
    use crate::data_config::{VectorColorMap, VectorComponents};
    let invalid = |reason: crate::RuleKind| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        kind: reason,
    };
    style.validate().map_err(|reason| invalid(reason))?;
    let grid = match components {
        VectorComponents::Points { u, v, w } => {
            validate_registered_column(pool, u)?;
            validate_registered_column(pool, v)?;
            // The z component is read only where the arrow is drawn in 3D;
            // on a flat chart it is kept and unused (#211).
            if let Some(w) = w
                .as_deref()
                .filter(|_| config.drawn_z_column(item).is_some())
            {
                validate_registered_column(pool, w)?;
            }
            None
        }
        VectorComponents::Grid { u, v } => {
            for column in u.columns.iter().chain(&v.columns) {
                validate_registered_column(pool, column)?;
            }
            if u.orientation != v.orientation || u.grid_layout != v.grid_layout {
                return Err(invalid(crate::RuleKind::VectorGridLayout));
            }
            Some(u)
        }
    };
    match (style.color_map.as_ref(), grid) {
        (None, _) => {}
        (Some(VectorColorMap::Column(column)), None) => {
            validate_registered_column(pool, column)?;
        }
        (Some(VectorColorMap::Grid(matrix)), Some(u)) => {
            for column in &matrix.columns {
                validate_registered_column(pool, column)?;
            }
            if matrix.orientation != u.orientation || matrix.grid_layout != u.grid_layout {
                return Err(invalid(crate::RuleKind::VectorColorGridLayout));
            }
        }
        (Some(_), _) => {
            return Err(invalid(crate::RuleKind::VectorColorMapForm));
        }
    }
    if style.color_map.is_some() && config.colorbar.is_none() {
        return Err(invalid(crate::RuleKind::VectorColorMapNeedsColorbar));
    }
    Ok(())
}

/// A line fill's declaration must resolve before any GPU work: a finite
/// baseline, or a referenced series that exists in the same chart, is not the
/// series itself, shares its x column and has a registered y column. A
/// different x column would need resampling — renderer-derived data — so it
/// is an error, not a fallback.
pub(in crate::renderer) fn validate_line_fill<'s>(
    pool: &impl ColumnRegistration,
    series: impl IntoIterator<Item = &'s SeriesConfig>,
    item: &SeriesConfig,
) -> Result<()> {
    let Some(line) = extract_line(&item.render_type) else {
        return Ok(());
    };
    let invalid = |reason: crate::RuleKind| FiggyError::InvalidSeriesConfig {
        series_id: item.series_id.clone(),
        kind: reason,
    };
    match line.fill.as_ref() {
        None => {}
        Some(LineFill::ToBaseline(baseline)) => {
            if !baseline.is_finite() {
                return Err(invalid(crate::RuleKind::LineFillBaseline));
            }
        }
        Some(LineFill::ToSeries(target)) => {
            if *target == item.series_id {
                return Err(invalid(crate::RuleKind::LineFillSelf));
            }
            let target = series
                .into_iter()
                .find(|other| other.series_id == *target)
                .ok_or_else(|| {
                    invalid(crate::RuleKind::LineFillTargetMissing {
                        target: target.to_string(),
                    })
                })?;
            if target.x_column != item.x_column {
                return Err(invalid(crate::RuleKind::LineFillXColumn {
                    target: target.series_id.clone(),
                    x_column: item.x_column.clone(),
                    target_x_column: target.x_column.clone(),
                }));
            }
            validate_registered_column(pool, &target.y_column)?;
        }
    }
    Ok(())
}
