//! Column registration checks and the walk over the columns a series reads.

use super::*;

pub(in crate::renderer) trait ColumnRegistration {
    fn column_len(&self, id: &str) -> Option<u64>;

    /// Whether `id` names a resident record (a box plot's `y_column`) or a
    /// resident ragged column (its outliers). Records and ragged columns are
    /// resident only.
    fn contains_composite(&self, id: &str) -> bool;

    fn contains_column(&self, id: &str) -> bool {
        self.column_len(id).is_some()
    }

    /// The uniform step of resident column `id`'s values
    /// (`ColumnSlot::uniform_step`, #134); `None` for a streamed column.
    fn uniform_step(&self, id: &str) -> Option<f64>;

    /// The statistics of resident triangle index column `id` (#210); `None`
    /// when `id` is not one. Triangle columns are resident only.
    fn triangle_stats(&self, id: &str) -> Option<crate::data_render::TriangleStats>;
}

impl ColumnRegistration for ColumnPool {
    /// A record's or ragged column's member columns are not columns a series
    /// can reference: they belong to their record and change id on every
    /// replacement.
    fn column_len(&self, id: &str) -> Option<u64> {
        self.slot(id)
            .filter(|_| !self.is_member_column(id))
            .map(|slot| slot.len_values as u64)
    }

    fn contains_composite(&self, id: &str) -> bool {
        self.record(id).is_some() || self.ragged(id).is_some()
    }

    fn uniform_step(&self, id: &str) -> Option<f64> {
        self.slot(id)
            .filter(|_| !self.is_member_column(id))
            .and_then(|slot| slot.uniform_step)
    }

    fn triangle_stats(&self, id: &str) -> Option<crate::data_render::TriangleStats> {
        ColumnPool::triangle_stats(self, id)
    }
}

impl ColumnRegistration for (&ColumnPool, &HashMap<ColumnId, RegisteredStreamColumn>) {
    fn column_len(&self, id: &str) -> Option<u64> {
        self.1
            .get(id)
            .map(|source| source.len)
            .or_else(|| self.0.column_len(id))
    }

    fn contains_composite(&self, id: &str) -> bool {
        self.0.contains_composite(id)
    }

    fn uniform_step(&self, id: &str) -> Option<f64> {
        if self.1.contains_key(id) {
            return None;
        }
        self.0.uniform_step(id)
    }

    fn triangle_stats(&self, id: &str) -> Option<crate::data_render::TriangleStats> {
        self.0.triangle_stats(id)
    }
}

pub(in crate::renderer) fn validate_registered_column(
    pool: &impl ColumnRegistration,
    id: &str,
) -> Result<()> {
    if pool.contains_column(id) {
        Ok(())
    } else {
        Err(FiggyError::UnknownColumn { id: id.to_owned() })
    }
}

pub(in crate::renderer) fn validate_axis_view_state(
    field: &'static str,
    axis: &AxisViewState,
) -> Result<()> {
    if !axis.min.is_finite() || !axis.max.is_finite() {
        return Err(FiggyError::InvalidConfig {
            field,
            kind: crate::RuleKind::AxisBoundsNotFinite,
        });
    }
    if axis.max <= axis.min {
        return Err(FiggyError::InvalidConfig {
            field,
            kind: crate::RuleKind::AxisBoundsOrder,
        });
    }
    if matches!(axis.scale, AxisScale::Logarithmic) && axis.min <= 0.0 {
        return Err(FiggyError::InvalidConfig {
            field,
            kind: crate::RuleKind::LogAxisBounds,
        });
    }
    if matches!(axis.scale, AxisScale::Probability(_)) && !(axis.min > 0.0 && axis.max < 1.0) {
        return Err(FiggyError::InvalidConfig {
            field,
            kind: crate::RuleKind::ProbabilityAxisBounds,
        });
    }
    Ok(())
}

pub(in crate::renderer) fn validate_chart_view_state(view: &ChartViewState) -> Result<()> {
    if view.chart_area.width == 0 || view.chart_area.height == 0 {
        return Err(FiggyError::InvalidChartArea {
            width: view.chart_area.width,
            height: view.chart_area.height,
        });
    }
    validate_axis_view_state("top_x", &view.top_x)?;
    validate_axis_view_state("bottom_x", &view.bottom_x)?;
    validate_axis_view_state("left_y", &view.left_y)?;
    validate_axis_view_state("right_y", &view.right_y)?;
    for axis in &view.overlay_y {
        validate_axis_view_state("overlays.y_axis", axis)?;
    }
    for axis in &view.overlay_x {
        validate_axis_view_state("overlays.x_axis", axis)?;
    }
    for subplot in &view.subplots {
        validate_chart_view_state(subplot)?;
    }
    Ok(())
}

pub(in crate::renderer) fn validate_error_ref_columns(
    pool: &impl ColumnRegistration,
    error: &ErrorRef,
) -> Result<()> {
    match error {
        ErrorRef::Symmetric { column } => validate_registered_column(pool, column),
        ErrorRef::Asymmetric { lower, upper } => {
            validate_registered_column(pool, lower)?;
            validate_registered_column(pool, upper)
        }
    }
}

/// Whether this series reads `id` from the pool.
///
/// Asks [`visit_series_columns`] rather than enumerating the columns a second
/// time. The two used to be hand-kept copies of the same list, which is a live
/// hazard without any new variant at all: give an existing variant another
/// column-bearing field, update one function and not the other, and column
/// removal and validation quietly disagree about what a series needs. One
/// enumeration cannot disagree with itself.
pub(in crate::renderer) fn series_references_column(series: &SeriesConfig, id: &str) -> bool {
    let mut found = false;
    visit_series_columns(series, &mut |column| found |= column == id);
    found
}

/// Whether removing column `id` removes series `series` from its chart: a
/// series that reads it, but for a monotonic series (#271), whose columns
/// are its envelope, written and removed by the renderer alone. Without
/// them it has no data and draws nothing until its group's next full call.
pub(in crate::renderer) fn series_dropped_with_column(series: &SeriesConfig, id: &str) -> bool {
    series.monotonic().is_none() && series_references_column(series, id)
}

pub(in crate::renderer) fn data_series_declarations_equal(
    left: &[SeriesConfig],
    right: &[SeriesConfig],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.series_id == right.series_id
                && left.source_id == right.source_id
                && left.x_column == right.x_column
                && left.y_column == right.y_column
                // A 3D series' height column is data too (#132, #211).
                && left.z_column == right.z_column
                && left.render_type == right.render_type
        })
}

/// Whether any of `ids` is read by any series in the list.
///
/// Takes an iterator rather than a slice so a caller that only has the ids as
/// part of a larger structure — a batch of `(id, source)` pairs, for one — does
/// not have to collect them into a `Vec` first.
pub(in crate::renderer) fn series_list_references_any_column<'i>(
    series: &[SeriesConfig],
    mut ids: impl Iterator<Item = &'i str>,
) -> bool {
    ids.any(|id| {
        series
            .iter()
            .any(|series| series_references_column(series, id))
    })
}

pub(in crate::renderer) fn try_insert_column_id(
    ids: &mut std::collections::HashSet<ColumnId>,
    id: &str,
    resource: &'static str,
) -> Result<bool> {
    if ids.contains(id) {
        return Ok(false);
    }
    ids.try_reserve(1)
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource,
            reason: error.to_string(),
        })?;
    let mut owned = String::new();
    owned
        .try_reserve_exact(id.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource,
            reason: error.to_string(),
        })?;
    owned.push_str(id);
    Ok(ids.insert(owned))
}

pub(in crate::renderer) fn prepare_chart_replacement_plan(
    chart_states: &HashMap<ChartId, ChartRenderState>,
    chart_order: &[ChartId],
    monotonic_order: &[ChartId],
    visual_revision: RenderRevision,
    target_id: ChartId,
    config: Config,
    series: Vec<SeriesConfig>,
    replaced: &[&str],
) -> Result<ChartReplacementPlan> {
    chart_states
        .get(&target_id)
        .ok_or(FiggyError::UnknownChart { id: target_id })?;
    let mut revisions = Vec::new();
    revisions
        .try_reserve(chart_order.len() + monotonic_order.len())
        .map_err(|error| FiggyError::StateAllocationFailed {
            resource: "chart replacement revisions",
            reason: error.to_string(),
        })?;
    for id in chart_order.iter().chain(monotonic_order) {
        let state = chart_states
            .get(id)
            .ok_or(FiggyError::UnknownChart { id: *id })?;
        let is_target = *id == target_id;
        if !is_target && !series_list_references_any_column(&state.series, replaced.iter().copied())
        {
            continue;
        }

        let ChartRevisions {
            desired,
            config: config_revision,
            series: series_revision,
            data,
            view,
            selection,
            raster,
            annotation,
        } = state.revisions;
        let desired = desired.successor("chart desired revision")?;
        let next = if is_target {
            ChartRevisions {
                desired,
                config: config_revision.successor("chart config revision")?,
                selection: selection_config_revision(selection, &state.config, &config)?,
                series: series_revision.successor("chart series revision")?,
                data: if !data_series_declarations_equal(&state.series, &series)
                    || series_list_references_any_column(&state.series, replaced.iter().copied())
                    || series_list_references_any_column(&series, replaced.iter().copied())
                {
                    data.successor("chart data revision")?
                } else {
                    data
                },
                view: if !stream_config_equal(&state.config, &config, &state.series) {
                    view.successor("chart view revision")?
                } else {
                    view
                },
                raster: raster.successor("chart raster revision")?,
                annotation: annotation_config_revision(annotation, &state.config, &config)?,
            }
        } else {
            // A chart that only reads the replaced columns: its data changed.
            ChartRevisions {
                desired,
                config: config_revision,
                selection,
                series: series_revision,
                data: data.successor("chart data revision")?,
                view,
                raster,
                annotation,
            }
        };
        revisions.push(ChartReplacementRevision {
            id: *id,
            revisions: next,
        });
    }
    if !revisions.iter().any(|update| update.id == target_id) {
        return Err(FiggyError::UnknownChart { id: target_id });
    }
    let visual = visual_revision.successor("renderer visual revision")?;
    Ok(ChartReplacementPlan {
        target_id,
        config,
        series,
        revisions,
        visual,
    })
}

/// Every pool column this series reads, visited once per occurrence.
///
/// The single enumeration: column removal, validation, pick invalidation, and
/// the derived-data stamps all reach the pool through here, so a column this
/// function does not name is a column the renderer believes nobody uses. The
/// list is the model's ([`SeriesConfig::visit_columns`] and
/// [`SeriesConfig::visit_composite_columns`]), whose exhaustive match makes a
/// new kind's columns a compile-time question.
pub(in crate::renderer) fn visit_series_columns(
    series: &SeriesConfig,
    visit: &mut impl FnMut(&str),
) {
    // The model owns the list (#160); the renderer reads records and ragged
    // columns too.
    series.visit_columns(&mut |id| visit(id));
    series.visit_composite_columns(&mut |id| visit(id));
}
