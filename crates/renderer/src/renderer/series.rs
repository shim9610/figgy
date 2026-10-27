//! Reading a series declaration: which primitives it draws, its columns and
//! error references, the pick geometry it implies, and the draw info reported
//! from the column pool.

use super::*;

/// Whether the line `series` draws under `draw_style` walks its arc prefix:
/// a dashed stroke takes its dash phase from it, a sketch stroke its wobble
/// and a streamline series' arrowheads their slots. A Milkyway ribbon is
/// never dashed and its stars are picked by their envelope, so it needs
/// none.
pub(super) fn line_pick_walks_arc(draw_style: &DrawStyle, series: &SeriesConfig) -> bool {
    if !effective_series_primitives(draw_style, &series.render_type).line {
        return false;
    }
    if series
        .render_type
        .streamlines()
        .is_some_and(|streamlines| streamlines.arrows.is_some())
    {
        return true;
    }
    let Some(line) = extract_line(&series.render_type) else {
        return false;
    };
    match draw_style {
        DrawStyle::Milkyway(_) => false,
        DrawStyle::Sketch(_) => true,
        DrawStyle::Precise | DrawStyle::Constellation(_) => {
            !matches!(line.line_style, LineStylePreset::Solid)
        }
    }
}

// Which primitives a render type draws.
//
// Derived from the extractors rather than listed again, following `has_errorbar`
// below. An exhaustive `match` in each predicate would make the compiler stop on
// a new variant — but it would only force the arms to be *written*, not to agree
// with the config lookup. Someone answering `Bar { .. } => true` in `has_scatter`
// while `extract_scatter` correctly says `None` reproduces the original bug
// exactly, past a compiler that saw nothing wrong. Asking the extractor makes the
// two impossible to disagree, and leaves exactly one exhaustive match per
// primitive as the place a new variant has to be decided.
//
// The bug this shape closes: `has_scatter` was
// `!matches!(rt, Line { .. })` — "anything that is not a bare line has points" —
// which answers *yes* for a variant that draws bars or a filled field. Nothing
// panics on that answer, which is what makes it dangerous:
// `ensure_precise_variants_for_items` and `build_series_layers` read this same
// predicate, so they agree, compile the scatter pipeline, and draw a
// `ColumnScatterLayer` for the series. With no scatter config to read,
// `create_style_for_series_scaled` falls back to
// `PrimitiveStyle::from_color(BLACK)` — a 4 px filled circle — so the new series
// silently gains black dots at every data point instead of crashing.
pub(super) fn has_line(rt: &DataRenderType) -> bool {
    extract_line(rt).is_some()
}

pub(super) fn has_scatter(rt: &DataRenderType) -> bool {
    extract_scatter(rt).is_some()
}

pub(super) fn has_errorbar(rt: &DataRenderType) -> bool {
    extract_err_x(rt).is_some() || extract_err_y(rt).is_some()
}

/// The line a render type draws through `(x_column, y_column)`, borrowed
/// from its config or, for a radar series, the polar line its outline and
/// fill are drawn as ([`crate::data_config::RadarRef::line_style`]): a
/// display mapping of the series' style, never of its data.
pub(super) fn extract_line(rt: &DataRenderType) -> Option<Cow<'_, DataLineStyleConfig>> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(Cow::Borrowed(line)),
        DataRenderType::Radar { .. } => rt.radar().map(|radar| Cow::Owned(radar.line_style())),
        // Streamlines are drawn by the line path: their polylines, NaN rows
        // breaking them, are this line.
        DataRenderType::Streamlines { line, .. } => Some(Cow::Borrowed(line)),
        DataRenderType::Scatter { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        // A contour's `contour.line` is not this line. This one is the polyline
        // through `(x_column, y_column)`; a contour is an implicit level set
        // drawn directly from the matrix. Returning its style here would make `has_line`
        // true and draw a polyline through the grid's coordinate columns.
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. } => None,
    }
}

/// The markers a render type draws at `(x_column, y_column)`, borrowed from
/// its config or, for a radar series with a marker, its vertex markers
/// ([`crate::data_config::RadarRef::scatter_style`]).
pub(super) fn extract_scatter(rt: &DataRenderType) -> Option<Cow<'_, DataScatterStyleConfig>> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(Cow::Borrowed(scatter)),
        DataRenderType::Radar { .. } => rt
            .radar()
            .and_then(|radar| radar.scatter_style())
            .map(Cow::Owned),
        // None of these draw points. This is the predicate `has_scatter` is
        // derived from, so answering otherwise is what put black 4 px dots at
        // every data point — see the note above.
        DataRenderType::Line { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        // Arrows are not markers: they have their own pipeline, pick radius
        // and legend mark (`extract_vector`).
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        // Arrowheads are not markers either; they are drawn along the line
        // by the vector glyph (`DataRenderType::streamlines`).
        | DataRenderType::Streamlines { .. } => None,
    }
}

/// What one series would draw right now, and whether the data forced a smaller
/// extent than the declaration asked for.
///
/// Column lengths are facts that arrive with the data, not SSoT — so a mismatch
/// never errors and never stops the draw: the smallest common extent is drawn
/// and reported here. This is the **series-common** window onto
/// that: a histogram's `edges` / `counts` truncation, a line's `min(x, y)`, and
/// (from W6) a matrix' grid extent all come out of the same query, so no
/// primitive ends up without a reporting path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesDrawInfo {
    /// Primitive instances the paint phase issues for this series: points,
    /// line segments, errorbars, or bars.
    pub drawn_count: u64,
    /// Grid columns actually used, for a matrix-backed series. `None` for every
    /// other render type.
    pub cols: Option<u64>,
    /// Grid rows actually used, for a matrix-backed series. `None` otherwise.
    pub rows: Option<u64>,
    /// True when some role column carries more values than `drawn_count` can
    /// consume — the declaration asked for more than the data supports, and the
    /// surplus is not drawn.
    pub truncated: bool,
}

/// The bar appearance of a bar-backed render type.
///
/// The single exhaustive match that decides whether a variant draws bars;
/// `has_bar` is derived from it so the predicate and the config lookup cannot
/// disagree, following `has_line` / `has_scatter`.
pub(super) fn extract_bar(rt: &DataRenderType) -> Option<&crate::data_config::DataBarStyleConfig> {
    match rt {
        DataRenderType::Histogram { bar } => Some(bar),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn has_bar(rt: &DataRenderType) -> bool {
    extract_bar(rt).is_some()
}

/// The fill appearance of a field-backed render type.
///
/// `Contour` is deliberately absent: contour lines over an unfilled field is a
/// distinct look, and giving it a default fill would paint something the config
/// does not ask for. The single exhaustive match that decides whether a variant
/// paints cells; `has_field` is derived from it.
pub(super) fn extract_field_fill(
    rt: &DataRenderType,
) -> Option<&crate::data_config::FieldFillConfig> {
    match rt {
        DataRenderType::Heatmap { fill, .. } | DataRenderType::HeatmapContour { fill, .. } => {
            Some(fill)
        }
        DataRenderType::Contour { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn has_field(rt: &DataRenderType) -> bool {
    extract_field_fill(rt).is_some()
}

/// Whether this render type draws implicit contour lines. A `Heatmap` with no contour
/// declaration does not, even though its banded fill can quantize into levels.
pub(super) fn has_contour(rt: &DataRenderType) -> bool {
    extract_contour(rt).is_some()
}

/// The contour declaration of a render type that draws contour lines. Its levels
/// also divide the bands of a quantized fill.
pub(crate) fn extract_contour(rt: &DataRenderType) -> Option<&crate::data_config::ContourConfig> {
    match rt {
        DataRenderType::Contour { contour, .. }
        | DataRenderType::HeatmapContour { contour, .. } => Some(contour),
        DataRenderType::Heatmap { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

/// The components and style of a vector series.
///
/// The single exhaustive match that decides whether a variant draws arrows;
/// `has_vector` is derived from it, following `extract_bar` / `extract_scatter`.
pub(super) fn extract_vector(
    rt: &DataRenderType,
) -> Option<(
    &crate::data_config::VectorComponents,
    &crate::data_config::VectorStyle,
)> {
    match rt {
        DataRenderType::Vector { components, style } => Some((components, style)),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn has_vector(rt: &DataRenderType) -> bool {
    extract_vector(rt).is_some()
}

/// `SeriesDrawInfo` for one series against current logical column lengths.
///
/// Exhaustive over the render types so a new primitive cannot ship without
/// deciding how its extent is reported. An unregistered column counts
/// as zero: nothing draws, and that is reported rather than raised — checking
/// registration is `validate_renderer_series`' job.
pub(super) fn series_draw_info_from_pool(
    pool: &impl ColumnRegistration,
    series: &SeriesConfig,
) -> SeriesDrawInfo {
    let len = |id: &str| -> u64 { pool.column_len(id).unwrap_or(0) };
    let x = len(&series.x_column);
    let y = len(&series.y_column);
    // Index-aligned roles: the shortest decides, and anything longer is surplus.
    let paired = |lengths: &[u64]| -> (u64, bool) {
        let low = lengths.iter().copied().min().unwrap_or(0);
        (low, lengths.iter().any(|value| *value > low))
    };

    match &series.render_type {
        // A polyline draws one segment between adjacent points.
        DataRenderType::Line { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count.saturating_sub(1),
                cols: None,
                rows: None,
                truncated,
            }
        }
        // Streamlines are a polyline too, NaN separators included; a colour
        // column is index-aligned with the coordinates and bounds them.
        DataRenderType::Streamlines { color_map, .. } => {
            let mut lengths = [x, y, 0];
            let used = match color_map {
                Some(map) => {
                    lengths[2] = len(&map.column);
                    3
                }
                None => 2,
            };
            let (count, truncated) = paired(&lengths[..used]);
            SeriesDrawInfo {
                drawn_count: count.saturating_sub(1),
                cols: None,
                rows: None,
                truncated,
            }
        }
        DataRenderType::Scatter { .. } | DataRenderType::ScatterLine { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. } => {
            // x, y, and up to two lo/hi pairs — the same six roles the
            // errorbar draw call minimizes over, on the stack.
            let mut lengths = [x, y, 0, 0, 0, 0];
            let mut used = 2;
            for error in [
                extract_err_x(&series.render_type),
                extract_err_y(&series.render_type),
            ]
            .into_iter()
            .flatten()
            {
                match error {
                    ErrorRef::Symmetric { column } => {
                        lengths[used] = len(column);
                        used += 1;
                    }
                    ErrorRef::Asymmetric { lower, upper } => {
                        lengths[used] = len(lower);
                        lengths[used + 1] = len(upper);
                        used += 2;
                    }
                }
            }
            let (count, truncated) = paired(&lengths[..used]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        // `n` bars need `n + 1` edges. Orientation decides which column is
        // which — never the lengths.
        DataRenderType::Histogram { bar } => {
            let (edges, values) = match bar.orientation {
                crate::data_config::BarOrientation::Vertical => (x, y),
                crate::data_config::BarOrientation::Horizontal => (y, x),
            };
            let bins = edges.saturating_sub(1);
            SeriesDrawInfo {
                drawn_count: bins.min(values),
                cols: None,
                rows: None,
                truncated: bins != values,
            }
        }
        // A field's primitive count is its cells. `cols` / `rows` report the
        // grid the declaration and the data agreed on, which is the only way a
        // host can tell a 10x10 grid drawn as 10x9 from one drawn whole.
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => {
            let extent = matrix_extent(matrix, x as usize, y as usize, |id| len(id) as usize);
            SeriesDrawInfo {
                drawn_count: (extent.cols as u64).saturating_mul(extent.rows as u64),
                cols: Some(extent.cols as u64),
                rows: Some(extent.rows as u64),
                truncated: extent.truncated,
            }
        }
        // One box per row of the position column; the record holds one row
        // per category (checked when the boxes are prepared).
        DataRenderType::BoxPlot { .. } => {
            let (count, truncated) = paired(&[x]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        // A radar polygon: one closed segment and one marker per vertex of
        // the category index and value columns, which hold one value per
        // category (checked when the polygon is prepared).
        DataRenderType::Radar { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        // One bar per row of the position and value columns; both hold one
        // value per category (checked when the bars are prepared).
        DataRenderType::CategoryBars { .. } => {
            let (count, truncated) = paired(&[x, y]);
            SeriesDrawInfo {
                drawn_count: count,
                cols: None,
                rows: None,
                truncated,
            }
        }
        // Point vectors: one arrow per index-aligned row of x, y, u, v and the
        // colour column. Grid vectors: one arrow per cell of the smallest
        // common extent of the u, v and colour grids.
        DataRenderType::Vector { components, style } => {
            let color_column = match style.color_map.as_ref() {
                Some(crate::data_config::VectorColorMap::Column(column)) => Some(len(column)),
                _ => None,
            };
            match components {
                crate::data_config::VectorComponents::Points { u, v } => {
                    let mut lengths = [x, y, len(u), len(v), 0];
                    let used = if let Some(color) = color_column {
                        lengths[4] = color;
                        5
                    } else {
                        4
                    };
                    let (count, truncated) = paired(&lengths[..used]);
                    SeriesDrawInfo {
                        drawn_count: count,
                        cols: None,
                        rows: None,
                        truncated,
                    }
                }
                crate::data_config::VectorComponents::Grid { u, v } => {
                    let extent = vector_grid_extent(u, v, style, x as usize, y as usize, |id| {
                        len(id) as usize
                    });
                    SeriesDrawInfo {
                        drawn_count: (extent.cols as u64).saturating_mul(extent.rows as u64),
                        cols: Some(extent.cols as u64),
                        rows: Some(extent.rows as u64),
                        truncated: extent.truncated,
                    }
                }
            }
        }
    }
}

/// The grid declaration of a matrix-backed render type.
///
/// The single exhaustive match that decides whether a variant is a field: both
/// "which columns does it reference" and "does it need a z scale" are derived
/// from this one answer instead of being listed again, so they cannot disagree
/// about a variant.
pub(super) fn extract_matrix(rt: &DataRenderType) -> Option<&crate::data_config::MatrixRef> {
    match rt {
        DataRenderType::Heatmap { matrix, .. }
        | DataRenderType::Contour { matrix, .. }
        | DataRenderType::HeatmapContour { matrix, .. } => Some(matrix),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::ScatterErrorbarXY { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarXY { .. }
        | DataRenderType::Histogram { .. }
        // A vector's grids are not "the matrix": they are components, not z.
        // `fit_matrix` reaches the `u` grid for the coordinate lattice.
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

/// Whether this render type cannot be drawn without `Config.colorbar`.
///
/// A field's z range and colormap live nowhere else, so without the colourbar
/// there is no value to paint — not a missing decoration, a missing input.
/// Derived from `extract_matrix` rather than matched again.
pub(super) fn requires_colorbar(rt: &DataRenderType) -> bool {
    extract_matrix(rt).is_some()
}

pub(super) fn extract_errorbar_style(rt: &DataRenderType) -> Option<&DataErrorBarStyleConfig> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_style, .. }
        | DataRenderType::ScatterErrorbarY { err_style, .. }
        | DataRenderType::ScatterErrorbarXY { err_style, .. }
        | DataRenderType::LineScatterErrorbarX { err_style, .. }
        | DataRenderType::LineScatterErrorbarY { err_style, .. }
        | DataRenderType::LineScatterErrorbarXY { err_style, .. } => Some(err_style),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn extract_err_y(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::ScatterErrorbarXY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_y, .. } => Some(err_y),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarX { .. }
        | DataRenderType::LineScatterErrorbarX { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn extract_err_x(rt: &DataRenderType) -> Option<&ErrorRef> {
    match rt {
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::ScatterErrorbarXY { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, .. } => Some(err_x),
        DataRenderType::Line { .. }
        | DataRenderType::Scatter { .. }
        | DataRenderType::ScatterLine { .. }
        | DataRenderType::ScatterErrorbarY { .. }
        | DataRenderType::LineScatterErrorbarY { .. }
        | DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Streamlines { .. } => None,
    }
}

pub(super) fn errorbar_direction_flags(rt: &DataRenderType) -> u32 {
    (u32::from(extract_err_y(rt).is_some()) * ERRORBAR_HAS_Y)
        | (u32::from(extract_err_x(rt).is_some()) * ERRORBAR_HAS_X)
}

pub(super) fn picked_ref_matches_series(series: &SeriesConfig, picked: &PickedPointRef) -> bool {
    if picked.series_id != series.series_id {
        return false;
    }
    match picked.source_id.as_deref() {
        Some(source_id) => series.source_id.as_deref() == Some(source_id),
        None => true,
    }
}

pub(super) fn picked_data_ref_matches_series(
    series: &SeriesConfig,
    picked: &PickedDataRef,
) -> bool {
    if picked.series_id() != series.series_id {
        return false;
    }
    match picked.source_id() {
        Some(source_id) => series.source_id.as_deref() == Some(source_id),
        None => true,
    }
}

/// Check that a radar series of the panel `config` holds one value per
/// category of the angle axis in both its category index column (`x_len`
/// values) and its value column (`y_len`): the column lengths are data, so
/// they are checked when the polygon is prepared.
pub(super) fn radar_columns_fit(
    config: &Config,
    series: &SeriesConfig,
    x_len: usize,
    y_len: usize,
) -> Result<()> {
    let n = config.bottom_x.category_count().unwrap_or(0);
    if x_len != n || y_len != n {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: format!(
                "a radar series holds one value per category: the angle axis has {n} \
                 categories, the category index column {:?} {x_len} values and the value \
                 column {:?} {y_len}",
                series.x_column, series.y_column
            ),
        });
    }
    Ok(())
}

/// Whether a radar series of `config` (one panel) closes its fill with the
/// segment from its last vertex back to the first: on a full circle. In a
/// sector the closing trapezoid would cover the others, so the fill is the
/// region between the open outline and the inner ring there, while the
/// outline still closes. The draw (`ColumnAreaLayer::closed`) and the pick
/// (the filled segment count) read this one answer.
pub(super) fn polar_fill_closes(config: &Config) -> bool {
    config
        .polar
        .as_ref()
        .is_some_and(|polar| polar.sector.is_none())
}

/// The categories a category bar series of `config` (one panel) draws
/// against: the categorical axis of the panel's bar direction.
pub(super) fn category_bar_categories(config: &Config) -> Option<&[crate::config::Category]> {
    let groups = config.bar_groups.clone().unwrap_or_default();
    match groups.direction {
        crate::data_config::CategoryBarDirection::Vertical => config.bottom_x.categories.as_deref(),
        crate::data_config::CategoryBarDirection::Horizontal => config.left_y.categories.as_deref(),
    }
}

/// The categories of `series`' bar axis, or the error a category bar series
/// without a categorical axis fails with.
fn category_bar_count(config: &Config, series: &SeriesConfig) -> Result<usize> {
    category_bar_categories(config)
        .map(<[_]>::len)
        .ok_or_else(|| FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason:
                "category bars need a categorical axis: bottom_x for vertical bars, left_y for horizontal ones"
                    .into(),
        })
}

/// The pool lane of a category bar column of `series` over `n` categories:
/// checked to hold one value per category.
fn category_bar_lane(
    series: &SeriesConfig,
    handle: &ColumnHandle,
    id: &str,
    n: usize,
) -> Result<u32> {
    if handle.len_values != n {
        return Err(FiggyError::InvalidSeriesConfig {
            series_id: series.series_id.clone(),
            reason: format!(
                "category bar column {id:?} has {} values for {n} categories; it needs one per category",
                handle.len_values
            ),
        });
    }
    u32::try_from(handle.offset / 4).map_err(|_| FiggyError::GpuResourceLimit {
        resource: "category bar column lane",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

/// Walk the members of `series`' bar group `group` (its category bar series,
/// in order): `visit(index, member, value column id, value column lane)`
/// for each, the lane checked by [`category_bar_lane`]. Any number of
/// members (#107 R3). Returns the member count.
fn visit_category_bar_members<'c>(
    config: &Config,
    group: impl IntoIterator<Item = &'c SeriesConfig>,
    series: &SeriesConfig,
    lookup: &mut impl FnMut(&str) -> Result<ColumnHandle>,
    mut visit: impl FnMut(usize, &'c SeriesConfig, &str, u32) -> Result<()>,
) -> Result<usize> {
    let groups = config.bar_groups.clone().unwrap_or_default();
    let n = category_bar_count(config, series)?;
    let mut members = 0usize;
    for item in group {
        if item.render_type.category_bars().is_none() {
            continue;
        }
        let (_, values) = item.category_bar_columns(groups.direction);
        let lane = category_bar_lane(series, &lookup(values)?, values, n)?;
        visit(members, item, values, lane)?;
        members += 1;
    }
    Ok(members)
}

/// Push the lane table of `series`, a category bar series of the panel
/// `config` whose bar group is `group`: every member's value column lane in
/// group order, `params.members` entries
/// ([`data_render::create_category_bar_data_bind_group`] writes them into
/// the table's mapping). [`category_bar_params`] has checked the same
/// members.
pub(super) fn category_bar_lanes<'c>(
    config: &Config,
    group: impl IntoIterator<Item = &'c SeriesConfig>,
    series: &SeriesConfig,
    mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    push: &mut dyn FnMut(u32),
) -> Result<()> {
    visit_category_bar_members(config, group, series, &mut lookup, |_, _, _, lane| {
        push(lane);
        Ok(())
    })
    .map(|_| ())
}

/// The params `series`, a category bar series of the panel `config` whose
/// bar group is `group` (the panel's category bar series in order), draws
/// with: its place in the group and the group's size, its own position
/// column lane, the group layout of `config.bar_groups` (already in render
/// pixels) and its corner radius and emphasis at `display_scale`. The
/// members' value column lanes are the series' lane table
/// ([`category_bar_lanes`]), sized by the group: a group holds any number
/// of series (#107 R3).
///
/// The columns are data, so they are checked here against the pool's
/// statistics: every column holds one value per category, and a percent
/// stack's value columns hold no negative value (`min_of`, the pool's
/// minimum; the error names the member series that has one, whichever
/// member is being prepared). `lookup` resolves a column to its handle.
pub(super) fn category_bar_params<'c>(
    config: &Config,
    group: impl IntoIterator<Item = &'c SeriesConfig>,
    series: &SeriesConfig,
    display_scale: f32,
    mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
    min_of: impl Fn(&str) -> Option<f64>,
) -> Result<data_render::CategoryBarParamsGpu> {
    use crate::data_config::{CategoryBarDirection, CategoryBarMode};
    let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
        series_id: series.series_id.clone(),
        reason,
    };
    let bars = series
        .render_type
        .category_bars()
        .ok_or_else(|| invalid("not a category bar series".into()))?;
    let groups = config.bar_groups.clone().unwrap_or_default();
    let n = category_bar_count(config, series)?;
    let mut params: data_render::CategoryBarParamsGpu = bytemuck::Zeroable::zeroed();
    let mut member = None;
    let members = visit_category_bar_members(
        config,
        group,
        series,
        &mut lookup,
        |index, item, values, _| {
            // The shader stacks a category's positive shares of its total; a
            // negative value would enter the total but not the stack and push
            // the shares past 100, so the pool's minimum refuses it here.
            if groups.mode == CategoryBarMode::PercentStacked
                && min_of(values).is_some_and(|min| min < 0.0)
            {
                return Err(FiggyError::InvalidSeriesConfig {
                    series_id: item.series_id.clone(),
                    reason: format!(
                        "a PercentStacked bar group needs non-negative values: the value column \
                         {values:?} of series {:?} has a negative value; draw it Stacked or \
                         Grouped instead",
                        item.series_id
                    ),
                });
            }
            if item.series_id == series.series_id {
                member = Some(index);
            }
            Ok(())
        },
    )?;
    let member = member.ok_or_else(|| invalid("the series is not in its bar group".into()))?;
    let members = u32::try_from(members).map_err(|_| FiggyError::GpuResourceLimit {
        resource: "category bar group members",
        requested: members as u64,
        limit: u64::from(u32::MAX),
    })?;
    let (positions, _) = series.category_bar_columns(groups.direction);
    params.positions = category_bar_lane(series, &lookup(positions)?, positions, n)?;
    params.count = n as u32;
    params.member = member as u32;
    params.members = members;
    params.mode = match groups.mode {
        CategoryBarMode::Grouped => data_render::CATEGORY_BAR_MODE_GROUPED,
        CategoryBarMode::Stacked => data_render::CATEGORY_BAR_MODE_STACKED,
        CategoryBarMode::PercentStacked => data_render::CATEGORY_BAR_MODE_PERCENT,
    };
    params.horizontal = u32::from(groups.direction == CategoryBarDirection::Horizontal);
    params.gap_px = groups.bar_gap.max(0.0);
    params.group_width = groups.group_width.clamp(0.0, 1.0);
    params.corner_radius_px = bars.style.corner_radius.max(0.0) * display_scale;
    params.emphasis = bars.style.emphasis_brightness;
    params.scale = display_scale;
    params.seed = crate::sketch::fnv1a(&series.series_id);
    Ok(params)
}

/// The params, the lane table and the vertex count a box plot series of the
/// panel `config` draws with: `group` is the panel's box plot series in order, `pool` the
/// column pool its record and outlier column live in, `lookup` resolves a
/// member column to its handle (so the panel's column fingerprint covers
/// them).
///
/// The record is data, so it is checked here against the pool: it is a
/// box plot record with one row per category, and the outlier column, when
/// the series names one, is a ragged column with one group per category.
/// The lane table is the record's field lanes in schema order
/// (`BOX_LANE_ABSENT` for an absent optional field), the outlier values and
/// offsets lanes, the group count and the longest group
/// (SHADER_COMMON.md §5.15); the box group holds any number of series
/// (#107 R3), each reading only its own record; the group layout of `config.bar_groups` is
/// already in render pixels (the display config is scaled) and the corner
/// radius is scaled to `display_scale`.
pub(super) fn box_plot_params<'c>(
    config: &Config,
    group: impl IntoIterator<Item = &'c SeriesConfig>,
    series: &SeriesConfig,
    display_scale: f32,
    pool: &ColumnPool,
    mut lookup: impl FnMut(&str) -> Result<ColumnHandle>,
) -> Result<(
    data_render::CategoryBarParamsGpu,
    [u32; data_render::BOX_PLOT_LANES],
    u32,
)> {
    use crate::data_config::CategoryBarDirection;
    use data_render::{
        BOX_LANE_ABSENT, BOX_LANE_MAX_GROUP, BOX_LANE_OUTLIER_GROUPS, BOX_LANE_OUTLIER_OFFSETS,
        BOX_LANE_OUTLIER_VALUES, BOX_PLOT_BASE_VERTICES, BOX_PLOT_LANES, BOX_PLOT_QUAD_VERTICES,
    };
    let invalid = |reason: String| FiggyError::InvalidSeriesConfig {
        series_id: series.series_id.clone(),
        reason,
    };
    let boxes = series
        .render_type
        .box_plot()
        .ok_or_else(|| invalid("not a box plot series".into()))?;
    let groups = config.bar_groups.clone().unwrap_or_default();
    let n = category_bar_categories(config)
        .map(<[_]>::len)
        .ok_or_else(|| {
            invalid(
                "box plots need a categorical axis: bottom_x for vertical boxes, left_y for horizontal ones"
                    .into(),
            )
        })?;
    let lane = |handle: &ColumnHandle| -> Result<u32> {
        u32::try_from(handle.offset / 4).map_err(|_| FiggyError::GpuResourceLimit {
            resource: "box plot column lane",
            requested: handle.offset / 4,
            limit: u64::from(u32::MAX),
        })
    };
    let mut params: data_render::CategoryBarParamsGpu = bytemuck::Zeroable::zeroed();
    let mut lanes = [0u32; BOX_PLOT_LANES];
    fn set(lanes: &mut [u32; BOX_PLOT_LANES], k: u32, value: u32) {
        lanes[k as usize] = value;
    }
    let (positions, record_id) = series.box_plot_columns();
    let record = pool.record(record_id).ok_or_else(|| {
        invalid(format!(
            "box plot record {record_id:?} is not resident: upload it with `upsert_record`"
        ))
    })?;
    if record.slot().schema != data_render::BOX_PLOT_RECORD {
        return Err(invalid(format!(
            "record {record_id:?} is not a box plot record (`BOX_PLOT_RECORD`)"
        )));
    }
    if record.slot().len as usize != n {
        return Err(invalid(format!(
            "box plot record {record_id:?} has {} rows for {n} categories; it needs one per category",
            record.slot().len
        )));
    }
    for k in 0..data_render::BOX_PLOT_RECORD.len() {
        let value = match record.slot().fields[k].as_deref() {
            Some(member) => lane(&lookup(member)?)?,
            None => BOX_LANE_ABSENT,
        };
        set(&mut lanes, k as u32, value);
    }
    let mut max_group = 0u32;
    match boxes.outliers {
        Some(id) => {
            let ragged = pool.ragged(id).ok_or_else(|| {
                invalid(format!(
                    "box plot outlier column {id:?} is not resident: upload it with `upsert_ragged`"
                ))
            })?;
            if ragged.slot().groups as usize != n {
                return Err(invalid(format!(
                    "box plot outlier column {id:?} has {} groups for {n} categories; it needs one per category",
                    ragged.slot().groups
                )));
            }
            set(
                &mut lanes,
                BOX_LANE_OUTLIER_VALUES,
                lane(&lookup(&ragged.slot().values)?)?,
            );
            set(
                &mut lanes,
                BOX_LANE_OUTLIER_OFFSETS,
                lane(&lookup(&ragged.slot().offsets)?)?,
            );
            set(&mut lanes, BOX_LANE_OUTLIER_GROUPS, ragged.slot().groups);
            max_group = ragged.stats().max_group;
            set(&mut lanes, BOX_LANE_MAX_GROUP, max_group);
        }
        None => {
            set(&mut lanes, BOX_LANE_OUTLIER_VALUES, BOX_LANE_ABSENT);
            set(&mut lanes, BOX_LANE_OUTLIER_OFFSETS, BOX_LANE_ABSENT);
        }
    }
    let mut member = None;
    let mut members = 0usize;
    for item in group {
        if item.render_type.box_plot().is_none() {
            continue;
        }
        if item.series_id == series.series_id {
            member = Some(members);
        }
        members += 1;
    }
    let member = member.ok_or_else(|| invalid("the series is not in its box group".into()))?;
    let positions = lookup(positions)?;
    if positions.len_values != n {
        return Err(invalid(format!(
            "box plot position column {:?} has {} values for {n} categories; it needs one per category",
            series.x_column, positions.len_values
        )));
    }
    params.positions = lane(&positions)?;
    params.count = n as u32;
    params.member = member as u32;
    params.members = members as u32;
    params.mode = data_render::CATEGORY_BAR_MODE_GROUPED;
    params.horizontal = u32::from(groups.direction == CategoryBarDirection::Horizontal);
    // The box gap is already in render pixels: `Config::scaled` scales
    // `bar_groups.bar_gap` with the display config, as for category bars.
    params.gap_px = groups.bar_gap.max(0.0);
    params.group_width = groups.group_width.clamp(0.0, 1.0);
    params.corner_radius_px = boxes.style.corner_radius.max(0.0) * display_scale;
    params.emphasis = boxes.style.emphasis_brightness;
    params.scale = display_scale;
    params.seed = crate::sketch::fnv1a(&series.series_id);
    let vertices = max_group
        .checked_mul(BOX_PLOT_QUAD_VERTICES)
        .and_then(|v| v.checked_add(BOX_PLOT_BASE_VERTICES))
        .ok_or_else(|| invalid("a box plot outlier group is too long to draw".into()))?;
    Ok((params, lanes, vertices))
}
