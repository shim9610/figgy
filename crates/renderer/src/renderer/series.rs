//! Reading a series declaration: which primitives it draws, its columns and
//! error references, the pick geometry it implies, and the draw info reported
//! from the column pool.

use super::*;

/// Whether the line `series` draws under `draw_style` walks its arc prefix:
/// a dashed stroke takes its dash phase from it and a sketch stroke its
/// wobble. A Milkyway ribbon is never dashed and its stars are picked by
/// their envelope, so it needs none.
pub(super) fn line_pick_walks_arc(draw_style: &DrawStyle, series: &SeriesConfig) -> bool {
    if !effective_series_primitives(draw_style, &series.render_type).line {
        return false;
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

pub(super) fn scatter_config_radius_px(
    scatter: &DataScatterStyleConfig,
    point_index: usize,
    use_style_mapping: bool,
) -> f32 {
    let mut radius = scatter.point_size;
    if use_style_mapping && scatter.point_style_index_column.is_none() {
        // A style table has no effect without an index column. Sparse
        // overrides are explicit by point index and can be resolved here.
        if let Some(overrides) = scatter.point_style_overrides.as_deref() {
            for ov in overrides {
                if ov.index == point_index
                    && let Some(size) = ov.style.point_size
                {
                    radius = size;
                }
            }
        }
    }
    radius.max(0.0)
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

pub(super) fn extract_line(rt: &DataRenderType) -> Option<&DataLineStyleConfig> {
    match rt {
        DataRenderType::Line { line }
        | DataRenderType::ScatterLine { line, .. }
        | DataRenderType::LineScatterErrorbarX { line, .. }
        | DataRenderType::LineScatterErrorbarY { line, .. }
        | DataRenderType::LineScatterErrorbarXY { line, .. } => Some(line),
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
        | DataRenderType::Vector { .. } => None,
    }
}

pub(super) fn extract_scatter(rt: &DataRenderType) -> Option<&DataScatterStyleConfig> {
    match rt {
        DataRenderType::Scatter { scatter }
        | DataRenderType::ScatterLine { scatter, .. }
        | DataRenderType::ScatterErrorbarX { scatter, .. }
        | DataRenderType::ScatterErrorbarY { scatter, .. }
        | DataRenderType::ScatterErrorbarXY { scatter, .. }
        | DataRenderType::LineScatterErrorbarX { scatter, .. }
        | DataRenderType::LineScatterErrorbarY { scatter, .. }
        | DataRenderType::LineScatterErrorbarXY { scatter, .. } => Some(scatter),
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::HeatmapContour { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
        | DataRenderType::Vector { .. } => None,
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
