//! Stepping a data selection across the points a series draws.

use super::*;

impl DataRenderType {
    /// Whether a series of this kind draws one selectable point per source
    /// index, the selection [`crate::config::Config::step_selection`] steps
    /// a [`PickedDataRef::Point`](crate::config::PickedDataRef::Point)
    /// across: every kind with a line or markers, a radar series (its
    /// vertices), a streamline series (its vertices) and a point vector (its
    /// anchors, which pick and highlight like markers).
    fn steps_points(&self) -> bool {
        match self {
            DataRenderType::Xy(_)
            | DataRenderType::Radar { .. }
            // Its samples, as a scatter or a line series'.
            | DataRenderType::Ternary { .. }
            // Its vertices, NaN separators included, as a line's (D4).
            | DataRenderType::Streamlines { .. }
            // Its rows (#141).
            | DataRenderType::Parallel { .. } => true,
            DataRenderType::Vector { components, .. } => {
                matches!(components, VectorComponents::Points { .. })
            }
            // Its raw points, when it has them.
            DataRenderType::BoxPlot { points, .. } => points.is_some(),
            DataRenderType::Histogram { .. }
            | DataRenderType::Field(_)
            | DataRenderType::RgbImage { .. }
            | DataRenderType::CategoryBars { .. }
            | DataRenderType::RangeBars { .. }
            // Its transitions step as points by their own rule (#203).
            | DataRenderType::EnergyLevels(_)
            // Its triangles step by their own rule (#210).
            | DataRenderType::TriMesh(_)
            // Its vertices and propagators step by their own rule (#234).
            | DataRenderType::Feynman(_)
            // Its kept points step through the renderer, which holds its
            // envelope (#271).
            | DataRenderType::MonotonicLine(_) => false,
        }
    }
}

impl crate::config::Config {
    /// Move the first typed data selection (`picked_data.refs[0]`) by
    /// source indices, over the series list `series` the chart draws, and
    /// return whether it moved. `column_len` gives the length of a
    /// registered column (`None` for a column that is not registered,
    /// which counts as empty). Indices clamp to the selected series; nothing
    /// wraps.
    ///
    /// - A point (of a series that draws points: lines, markers, radar
    ///   vertices, streamline vertices and point vectors) moves along its
    ///   series by `dx`, and `dy` moves to another point-drawing series, in
    ///   series order. A streamline's NaN separator rows count as points. A
    ///   point vector counts only the points where its `u` and `v` columns
    ///   have a value too.
    /// - A category bar moves along its categories by the delta of the
    ///   pointer axis its category axis runs along (`dx` for vertical bars,
    ///   `dy` for horizontal ones) and across the category bar series of its
    ///   layer by the other; a box moves the same way across the box plot
    ///   series of its layer (boxes, never their outliers), and a range bar
    ///   across the range bar series of its layer, by row.
    /// - A histogram bin moves along its bins by `dx` for vertical bars and
    ///   `dy` for horizontal ones, and across the histograms of the same
    ///   orientation by the other.
    /// - A matrix cell (a field's or a grid vector's, over its `u` grid)
    ///   moves by `dx` along x and `dy` along y.
    ///
    /// - On an energy-level series (#203), a level (a histogram bin) steps
    ///   by `dx` over the level rows its view draws and a transition (a
    ///   point) over the transitions it draws, and `dy` moves across the
    ///   energy-level series, in series order.
    /// - A mesh triangle (#210) steps by `dx` over the triangles of its
    ///   series and `dy` moves across the triangle mesh series, in series
    ///   order. For a triangle index column `column_len` answers its index
    ///   count, three per triangle.
    ///
    /// A contour level and a value label do not step. An error names a
    /// matrix cell selection whose series draws no matrix any more.
    pub fn step_selection(
        &mut self,
        series: &[SeriesConfig],
        dx: i32,
        dy: i32,
        column_len: impl Fn(&str) -> Option<usize>,
    ) -> Result<bool, crate::RuleKind> {
        let Some(next) = self.stepped_selection(series, dx, dy, &column_len)? else {
            return Ok(false);
        };
        let Some(current) = self.picked_data.as_mut().and_then(|s| s.refs.first_mut()) else {
            return Ok(false);
        };
        if *current == next {
            return Ok(false);
        }
        *current = next;
        Ok(true)
    }

    /// The selection [`Self::step_selection`] moves to, or `None` when it
    /// does not move.
    fn stepped_selection(
        &self,
        series: &[SeriesConfig],
        dx: i32,
        dy: i32,
        column_len: &dyn Fn(&str) -> Option<usize>,
    ) -> Result<Option<crate::config::PickedDataRef>, crate::RuleKind> {
        use crate::config::PickedDataRef;
        let Some(reference) = self.picked_data.as_ref().and_then(|s| s.refs.first()) else {
            return Ok(None);
        };
        let Some(position) = series
            .iter()
            .position(|s| s.series_id == reference.series_id())
        else {
            return Ok(None);
        };
        // An energy-level series steps over the rows of its SSOT (#203).
        if series[position].energy_levels().is_some() {
            return Ok(crate::energy_levels::stepped_selection(
                series, position, reference, dx, dy,
            ));
        }
        // A Feynman series steps over the rows of its SSOT (#234).
        if series[position].feynman().is_some() {
            return Ok(crate::feynman::stepped_selection(
                series, position, reference, dx, dy,
            ));
        }
        let len = |column: &str| column_len(column).unwrap_or(0);
        let shift = |index: usize, delta: i32, count: usize| {
            index
                .saturating_add_signed(delta as isize)
                .min(count.saturating_sub(1))
        };
        Ok(match reference {
            PickedDataRef::Point {
                series_id,
                point_index,
                ..
            } => {
                let points = || series.iter().filter(|s| s.render_type.steps_points());
                let Some(index) = points().position(|s| s.series_id == *series_id) else {
                    return Ok(None);
                };
                let Some(next) = points().nth(shift(index, dy, points().count())) else {
                    return Ok(None);
                };
                let mut count = match next.render_type.box_plot().and_then(|b| b.points) {
                    Some(points) => len(&points.positions).min(len(&points.values)),
                    None => len(&next.x_column).min(len(&next.y_column)),
                };
                // A point vector draws only where all four columns have a value.
                if let DataRenderType::Vector {
                    components: VectorComponents::Points { u, v, w },
                    ..
                } = &next.render_type
                {
                    count = count.min(len(u)).min(len(v));
                    // A 3D arrow needs its z component too (#211).
                    if let (Some(w), Some(_)) = (w, self.drawn_z_column(next)) {
                        count = count.min(len(w));
                    }
                }
                // A 3D point needs its height too (#207).
                if let Some(z) = self.drawn_z_column(next) {
                    count = count.min(len(z));
                }
                // A ternary sample needs its third component too.
                if let DataRenderType::Ternary { c_column, .. } = &next.render_type {
                    count = count.min(len(c_column));
                }
                // A parallel row needs every axis column (#141).
                if let DataRenderType::Parallel { columns, .. } = &next.render_type {
                    for column in columns {
                        count = count.min(len(column));
                    }
                }
                Some(PickedDataRef::Point {
                    series_id: next.series_id.clone(),
                    source_id: next.source_id.clone(),
                    point_index: shift(*point_index, dx, count),
                })
            }
            PickedDataRef::HistogramBin {
                series_id,
                bin_index,
                ..
            } => {
                let selected = &series[position].render_type;
                let boxes = selected.box_plot().is_some();
                let ranges = selected.range_bars().is_some();
                if selected.category_bars().is_some() || boxes || ranges {
                    let layer = self.layer_of_series(series_id);
                    let groups = self.bar_groups_of(layer);
                    let horizontal = groups.direction == CategoryBarDirection::Horizontal;
                    let members = || {
                        series.iter().filter(|s| {
                            (if boxes {
                                s.render_type.box_plot().is_some()
                            } else if ranges {
                                s.render_type.range_bars().is_some()
                            } else {
                                s.render_type.category_bars().is_some()
                            }) && self.layer_of_series(&s.series_id) == layer
                        })
                    };
                    let Some(index) = members().position(|s| s.series_id == *series_id) else {
                        return Ok(None);
                    };
                    let (bar_delta, series_delta) = if horizontal { (dy, dx) } else { (dx, dy) };
                    let Some(next) = members().nth(shift(index, series_delta, members().count()))
                    else {
                        return Ok(None);
                    };
                    // A box's record is not a registered column: its
                    // positions hold one value per category.
                    let count = if boxes {
                        len(next.box_plot_columns().0)
                    } else if let Some(bars) = next.render_type.range_bars() {
                        // A range bar is drawn where all its columns hold a
                        // value: the shortest of them bounds the bars.
                        bars.extra_columns()
                            .chain([next.x_column.as_str(), next.y_column.as_str()])
                            .map(len)
                            .min()
                            .unwrap_or(0)
                    } else {
                        let (positions, values) = next.category_bar_columns(groups.direction);
                        len(positions).min(len(values))
                    };
                    if count == 0 {
                        return Ok(None);
                    }
                    Some(PickedDataRef::HistogramBin {
                        series_id: next.series_id.clone(),
                        source_id: next.source_id.clone(),
                        bin_index: shift(*bin_index, bar_delta, count),
                        half: None,
                    })
                } else {
                    let DataRenderType::Histogram { bar } = selected else {
                        return Ok(None);
                    };
                    let horizontal = matches!(bar.orientation, BarOrientation::Horizontal);
                    let histograms = || {
                        series.iter().filter(|s| {
                            matches!(&s.render_type,
                                DataRenderType::Histogram { bar: other }
                                if other.orientation == bar.orientation)
                        })
                    };
                    let Some(index) = histograms().position(|s| s.series_id == *series_id) else {
                        return Ok(None);
                    };
                    let (bin_delta, series_delta) = if horizontal { (dy, dx) } else { (dx, dy) };
                    let Some(next) =
                        histograms().nth(shift(index, series_delta, histograms().count()))
                    else {
                        return Ok(None);
                    };
                    let (edges, values) = if horizontal {
                        (&next.y_column, &next.x_column)
                    } else {
                        (&next.x_column, &next.y_column)
                    };
                    let count = len(edges).saturating_sub(1).min(len(values));
                    if count == 0 {
                        return Ok(None);
                    }
                    Some(PickedDataRef::HistogramBin {
                        series_id: next.series_id.clone(),
                        source_id: next.source_id.clone(),
                        bin_index: shift(*bin_index, bin_delta, count),
                        half: None,
                    })
                }
            }
            PickedDataRef::MatrixCell {
                source_id,
                series_id,
                x_index,
                y_index,
            } => {
                let selected = &series[position];
                // A field steps over its matrix; a grid vector over its `u`
                // grid, which every other grid of the series matches.
                let matrix = match &selected.render_type {
                    DataRenderType::Field(crate::data_config::FieldSeries { matrix, .. }) => matrix,
                    // An image steps over its red grid, which the others match.
                    DataRenderType::RgbImage { red, .. } => red,
                    DataRenderType::Vector {
                        components: VectorComponents::Grid { u, .. },
                        ..
                    } => u,
                    _ => return Err(crate::RuleKind::SelectedMatrixGone),
                };
                let edges = usize::from(matches!(matrix.grid_layout, GridLayout::Edges));
                let depth = matrix
                    .columns
                    .iter()
                    .filter_map(|c| column_len(c))
                    .min()
                    .unwrap_or(0);
                let (nx, ny) = if matches!(matrix.orientation, MatrixOrientation::ColumnsAreY) {
                    (depth, matrix.columns.len())
                } else {
                    (matrix.columns.len(), depth)
                };
                Some(PickedDataRef::MatrixCell {
                    source_id: source_id.clone(),
                    series_id: series_id.clone(),
                    x_index: shift(
                        *x_index,
                        dx,
                        nx.min(len(&selected.x_column).saturating_sub(edges)),
                    ),
                    y_index: shift(
                        *y_index,
                        dy,
                        ny.min(len(&selected.y_column).saturating_sub(edges)),
                    ),
                })
            }
            PickedDataRef::MeshTriangle {
                series_id,
                triangle_index,
                ..
            } => {
                let meshes = || series.iter().filter(|s| s.tri_mesh().is_some());
                let Some(index) = meshes().position(|s| s.series_id == *series_id) else {
                    return Ok(None);
                };
                let Some(next) = meshes().nth(shift(index, dy, meshes().count())) else {
                    return Ok(None);
                };
                let Some(mesh) = next.tri_mesh() else {
                    return Ok(None);
                };
                let count = crate::tri_mesh::TriMeshSeries::triangle_count(len(&mesh.triangles));
                if count == 0 {
                    return Ok(None);
                }
                Some(PickedDataRef::MeshTriangle {
                    series_id: next.series_id.clone(),
                    source_id: next.source_id.clone(),
                    triangle_index: shift(*triangle_index, dx, count),
                })
            }
            PickedDataRef::ContourLevel { .. }
            | PickedDataRef::ValueLabel { .. }
            // A Feynman part on another kind of series names nothing to
            // step (#234).
            | PickedDataRef::FeynmanVertex { .. }
            | PickedDataRef::FeynmanPropagator { .. }
            | PickedDataRef::FeynmanLabel { .. } => None,
        })
    }
}
