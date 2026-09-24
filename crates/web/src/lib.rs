//! figgy for the web.
//!
//! The public browser surface is the `figgy-chart.js` Custom Element
//! (`<figgy-chart>`). It owns the canvas, async initialization, rAF loop,
//! resize/DPR handling, pointer wiring, busy gate, and CustomEvent output.
//! This Rust module exposes the raw `FiggyChart` wasm class that the Custom
//! Element uses as its low-level kernel; advanced hosts may call it directly
//! when they intentionally want to own those browser responsibilities.
//!
//! Lifecycle model: **one instance, register/unregister**. Columns and series
//! are managed by id. Column registration and explicit registration updates
//! are separate operations; `set_series` only selects registered columns for
//! drawing. `remove_*` unregisters (and drops dependents), and pool
//! defragmentation runs automatically after removals.
//!
//! The whole implementation is gated to `wasm32`: on native targets this
//! crate compiles to an empty library so `cargo test --workspace` stays
//! green. Build the artifact with:
//!
//! ```bash
//! npx wasm-pack build crates/web --release --target web
//! ```
//!
//! See `crates/renderer/WASM.md` for the I/O architecture this implements.

#[cfg(any(target_arch = "wasm32", test))]
mod borrowed_column;
#[cfg(any(target_arch = "wasm32", test))]
mod scalar_job;

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameDecision {
    Clean,
    MaintenanceOnly,
    Draw { refresh_raster: bool },
}

#[cfg(any(target_arch = "wasm32", test))]
fn consume_successful_frame(view_dirty: &mut bool, redraw_pending: &mut bool) {
    *view_dirty = false;
    *redraw_pending = false;
}

#[cfg(any(target_arch = "wasm32", test))]
fn frame_decision(
    renderer_dirty: bool,
    raster_dirty: bool,
    view_dirty: bool,
    redraw_pending: bool,
    maintenance_pending: bool,
) -> FrameDecision {
    let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
    match (visual_pending, maintenance_pending) {
        (false, false) => FrameDecision::Clean,
        (false, true) => FrameDecision::MaintenanceOnly,
        (true, _) => FrameDecision::Draw {
            refresh_raster: raster_dirty || view_dirty,
        },
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn picked_point_json_string(picked: &renderer::PickedPoint) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": picked.source_id.as_ref(),
        "series_id": &picked.series_id,
        "point_index": picked.point_index,
        "distance_px": picked.distance_px,
    }))
}

#[cfg(any(target_arch = "wasm32", test))]
fn picked_data_json_string(picked: &renderer::PickedData) -> serde_json::Result<String> {
    let value = match picked {
        renderer::PickedData::Point {
            source_id,
            series_id,
            point_index,
            distance_px,
        } => serde_json::json!({
            "kind": "point",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "point_index": point_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::HistogramBin {
            source_id,
            series_id,
            bin_index,
            distance_px,
        } => serde_json::json!({
            "kind": "histogram_bin",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "bin_index": bin_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::MatrixCell {
            source_id,
            series_id,
            x_index,
            y_index,
            distance_px,
        } => serde_json::json!({
            "kind": "matrix_cell",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "x_index": x_index,
            "y_index": y_index,
            "distance_px": distance_px,
        }),
        renderer::PickedData::ContourLevel {
            source_id,
            series_id,
            level_index,
            x_index,
            y_index,
            distance_px,
        } => serde_json::json!({
            "kind": "contour_level",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "level_index": level_index,
            "x_index": x_index,
            "y_index": y_index,
            "distance_px": distance_px,
        }),
    };
    serde_json::to_string(&value)
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RevisionedColumn {
    id: String,
    revision: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
/// `None` only for metadata-backed histogram fitting. Paired primitives and
/// matrix fields both produce a GPU request, with distinct normalized modes.
fn series_extent_mode(
    render_type: &renderer::DataRenderType,
) -> Option<renderer::GpuSeriesFitMode> {
    renderer::GpuSeriesFitMode::from_render_type(render_type)
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ErrorRefKind {
    Symmetric,
    Asymmetric,
}

/// Revision identity for one active error direction. The explicit kind keeps a
/// symmetric ref distinct from an asymmetric ref even when both name the same
/// physical column in both roles.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ErrorDirectionKey {
    kind: ErrorRefKind,
    lower: RevisionedColumn,
    upper: RevisionedColumn,
}

/// GPU fit-bound identity for one drawable series domain.
///
/// Inactive error directions are `None`; the renderer's internal filler
/// binding is deliberately excluded from this data identity.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SeriesExtentKey {
    mode: renderer::GpuSeriesFitMode,
    x: RevisionedColumn,
    y: RevisionedColumn,
    x_error: Option<ErrorDirectionKey>,
    y_error: Option<ErrorDirectionKey>,
    /// Matrix value columns affect the usable grid dimensions even though the
    /// fit reads only x/y coordinate values. Their ordered revisions therefore
    /// belong to the job identity.
    matrix: Vec<RevisionedColumn>,
    /// Orientation decides which resolved matrix dimension belongs to x/y.
    /// It is independent of the cell/sample lattice encoded by `mode`.
    field_orientation: Option<u8>,
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq)]
struct SeriesExtentRequest {
    key: SeriesExtentKey,
    series: renderer::SeriesConfig,
}

#[cfg(any(target_arch = "wasm32", test))]
type SeriesExtentJobMap =
    std::collections::HashMap<SeriesExtentKey, std::rc::Rc<crate::scalar_job::SeriesExtentJob>>;

#[cfg(any(target_arch = "wasm32", test))]
type SeriesExtentJobSelection = (SeriesExtentJobMap, Vec<SeriesExtentRequest>);

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
enum SeriesExtentRequestError {
    Allocation(std::collections::TryReserveError),
    MissingLiveRevision,
}

#[cfg(any(target_arch = "wasm32", test))]
impl std::fmt::Display for SeriesExtentRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allocation(error) => write!(formatter, "{error}"),
            Self::MissingLiveRevision => {
                formatter.write_str("series references a column without a live revision")
            }
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn checked_column_revision(next: u64) -> Result<(u64, u64), &'static str> {
    next.checked_add(1)
        .map(|successor| (next, successor))
        .ok_or("column revision counter exhausted")
}

#[cfg(any(target_arch = "wasm32", test))]
const DEMO_COLUMN_IDS: [&str; 4] = ["demo_x", "demo_sin", "demo_t", "demo_rc"];

#[cfg(any(target_arch = "wasm32", test))]
fn err_refs(
    rt: &renderer::DataRenderType,
) -> (
    Option<&renderer::data_config::ErrorRef>,
    Option<&renderer::data_config::ErrorRef>,
) {
    use renderer::DataRenderType;

    match rt {
        DataRenderType::Scatter { .. }
        | DataRenderType::Line { .. }
        | DataRenderType::ScatterLine { .. } => (None, None),
        DataRenderType::ScatterErrorbarX { err_x, .. }
        | DataRenderType::LineScatterErrorbarX { err_x, .. } => (Some(err_x), None),
        DataRenderType::ScatterErrorbarY { err_y, .. }
        | DataRenderType::LineScatterErrorbarY { err_y, .. } => (None, Some(err_y)),
        DataRenderType::ScatterErrorbarXY { err_x, err_y, .. }
        | DataRenderType::LineScatterErrorbarXY { err_x, err_y, .. } => (Some(err_x), Some(err_y)),
        // Bars and fields carry no error columns.
        DataRenderType::Histogram { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. } => (None, None),
    }
}

/// (lower, upper) error column ids of one ref. Symmetric refs use one column.
#[cfg(any(target_arch = "wasm32", test))]
fn err_cols(errors: &renderer::data_config::ErrorRef) -> (&str, &str) {
    use renderer::data_config::ErrorRef;

    match errors {
        ErrorRef::Symmetric { column } => (column, column),
        ErrorRef::Asymmetric { lower, upper } => (lower, upper),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn histogram_fit_columns<'a>(
    cfg: &'a renderer::SeriesConfig,
    orientation: &renderer::data_config::BarOrientation,
) -> (&'a str, &'a str) {
    if matches!(orientation, renderer::data_config::BarOrientation::Vertical) {
        (&cfg.x_column, &cfg.y_column)
    } else {
        (&cfg.y_column, &cfg.x_column)
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn revisioned_column_from(
    revisions: &std::collections::HashMap<String, u64>,
    id: &str,
) -> Option<RevisionedColumn> {
    Some(RevisionedColumn {
        id: id.to_string(),
        revision: *revisions.get(id)?,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn error_direction_key_from(
    revisions: &std::collections::HashMap<String, u64>,
    errors: &renderer::data_config::ErrorRef,
) -> Option<ErrorDirectionKey> {
    use renderer::data_config::ErrorRef;

    let (lower_id, upper_id) = err_cols(errors);
    Some(ErrorDirectionKey {
        kind: match errors {
            ErrorRef::Symmetric { .. } => ErrorRefKind::Symmetric,
            ErrorRef::Asymmetric { .. } => ErrorRefKind::Asymmetric,
        },
        lower: revisioned_column_from(revisions, lower_id)?,
        upper: revisioned_column_from(revisions, upper_id)?,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn series_extent_request_from(
    revisions: &std::collections::HashMap<String, u64>,
    cfg: &renderer::SeriesConfig,
) -> Option<SeriesExtentRequest> {
    let (err_x, err_y) = err_refs(&cfg.render_type);
    let x_error = err_x.and_then(|errors| error_direction_key_from(revisions, errors));
    let y_error = err_y.and_then(|errors| error_direction_key_from(revisions, errors));
    if err_x.is_some() != x_error.is_some() || err_y.is_some() != y_error.is_some() {
        return None;
    }

    let (matrix, field_orientation) = match &cfg.render_type {
        renderer::DataRenderType::Heatmap { matrix, .. }
        | renderer::DataRenderType::Contour { matrix, .. }
        | renderer::DataRenderType::HeatmapContour { matrix, .. } => (
            matrix
                .columns
                .iter()
                .map(|id| revisioned_column_from(revisions, id))
                .collect::<Option<Vec<_>>>()?,
            Some(match &matrix.orientation {
                renderer::data_config::MatrixOrientation::ColumnsAreX => 0,
                renderer::data_config::MatrixOrientation::ColumnsAreY => 1,
            }),
        ),
        renderer::DataRenderType::Scatter { .. }
        | renderer::DataRenderType::Line { .. }
        | renderer::DataRenderType::ScatterLine { .. }
        | renderer::DataRenderType::ScatterErrorbarX { .. }
        | renderer::DataRenderType::ScatterErrorbarY { .. }
        | renderer::DataRenderType::ScatterErrorbarXY { .. }
        | renderer::DataRenderType::LineScatterErrorbarX { .. }
        | renderer::DataRenderType::LineScatterErrorbarY { .. }
        | renderer::DataRenderType::LineScatterErrorbarXY { .. }
        | renderer::DataRenderType::Histogram { .. } => (Vec::new(), None),
    };
    Some(SeriesExtentRequest {
        key: SeriesExtentKey {
            mode: series_extent_mode(&cfg.render_type)?,
            x: revisioned_column_from(revisions, &cfg.x_column)?,
            y: revisioned_column_from(revisions, &cfg.y_column)?,
            x_error,
            y_error,
            matrix,
            field_orientation,
        },
        series: cfg.clone(),
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn active_series_extent_requests(
    series_cfgs: &[renderer::SeriesConfig],
    revisions: &std::collections::HashMap<String, u64>,
) -> Result<Vec<SeriesExtentRequest>, SeriesExtentRequestError> {
    use std::collections::HashSet;

    let capacity = series_cfgs.len();
    let mut active = HashSet::new();
    active
        .try_reserve(capacity)
        .map_err(SeriesExtentRequestError::Allocation)?;
    let mut ordered = Vec::new();
    ordered
        .try_reserve(capacity)
        .map_err(SeriesExtentRequestError::Allocation)?;

    for cfg in series_cfgs {
        if series_extent_mode(&cfg.render_type).is_none() {
            continue;
        }
        let request = series_extent_request_from(revisions, cfg)
            .ok_or(SeriesExtentRequestError::MissingLiveRevision)?;
        if active.insert(request.key.clone()) {
            ordered.push(request);
        }
    }
    Ok(ordered)
}

#[cfg(any(target_arch = "wasm32", test))]
fn series_extent_key_matches_config(
    key: &SeriesExtentKey,
    cfg: &renderer::SeriesConfig,
    revisions: &std::collections::HashMap<String, u64>,
) -> bool {
    series_extent_request_from(revisions, cfg).is_some_and(|request| request.key == *key)
}

#[cfg(any(target_arch = "wasm32", test))]
fn select_series_extent_job_map(
    ordered_requests: Vec<SeriesExtentRequest>,
    existing: &SeriesExtentJobMap,
    retry_failed: bool,
) -> Result<SeriesExtentJobSelection, std::collections::TryReserveError> {
    use std::collections::HashMap;

    let mut selected = HashMap::new();
    selected.try_reserve(ordered_requests.len())?;
    let mut pending = Vec::new();
    pending.try_reserve(ordered_requests.len())?;
    for request in ordered_requests {
        let value = match existing.get(&request.key) {
            Some(job) if !series_extent_needs_submission(Some(job.status()), retry_failed) => {
                std::rc::Rc::clone(job)
            }
            _ => {
                pending.push(request.clone());
                crate::scalar_job::SeriesExtentJob::pending()
            }
        };
        selected.insert(request.key, value);
    }
    Ok((selected, pending))
}

#[cfg(any(target_arch = "wasm32", test))]
fn retain_valid_series_extent_jobs(
    series_cfgs: &[renderer::SeriesConfig],
    revisions: &std::collections::HashMap<String, u64>,
    existing: &SeriesExtentJobMap,
) -> Result<SeriesExtentJobMap, SeriesExtentRequestError> {
    use std::collections::HashMap;

    let requests = active_series_extent_requests(series_cfgs, revisions)?;
    let mut retained = HashMap::new();
    retained
        .try_reserve(requests.len())
        .map_err(SeriesExtentRequestError::Allocation)?;
    for request in requests {
        if let Some(job) = existing.get(&request.key) {
            retained.insert(request.key, std::rc::Rc::clone(job));
        }
    }
    Ok(retained)
}

#[cfg(any(target_arch = "wasm32", test))]
fn series_extent_needs_submission(
    status: Option<crate::scalar_job::SeriesExtentStatus>,
    retry_failed: bool,
) -> bool {
    status.is_none()
        || retry_failed && status == Some(crate::scalar_job::SeriesExtentStatus::RetryableFailed)
}

#[cfg(any(target_arch = "wasm32", test))]
struct PreparedColumnMetadata {
    columns: std::collections::HashMap<String, usize>,
    revisions: std::collections::HashMap<String, u64>,
    next_revision: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ColumnRegistryAction {
    Register,
    Update,
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_registry_action(
    is_registered: bool,
    action: ColumnRegistryAction,
) -> Result<(), &'static str> {
    match (action, is_registered) {
        (ColumnRegistryAction::Register, false) | (ColumnRegistryAction::Update, true) => Ok(()),
        (ColumnRegistryAction::Register, true) => Err("is already registered"),
        (ColumnRegistryAction::Update, false) => Err("is not registered"),
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_data_len(len: usize) -> Result<(), &'static str> {
    if len == 0 {
        Err("data must not be empty")
    } else {
        Ok(())
    }
}

/// Validate a batch registration's shape before anything is uploaded.
///
/// The flat-buffer size check is an **error**, not a truncation: the
/// smallest-common-extent rule is about columns disagreeing with
/// each other, whereas a buffer whose size does not match the declared
/// `ids.len() x values_per_column` shape cannot be cut into columns at all.
#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_batch(
    columns: &std::collections::HashMap<String, usize>,
    ids: &[String],
    values_per_column: usize,
    data_len: usize,
) -> Result<(), String> {
    if ids.is_empty() {
        return Err("a column batch needs at least one id".to_string());
    }
    validate_column_data_len(values_per_column)
        .map_err(|reason| format!("values_per_column {reason}"))?;
    let expected = ids
        .len()
        .checked_mul(values_per_column)
        .ok_or_else(|| "column batch size overflows".to_string())?;
    if data_len != expected {
        return Err(format!(
            "column batch data has {data_len} values, but {} ids x {values_per_column} \
             values_per_column is {expected}",
            ids.len()
        ));
    }
    // One set rather than a quadratic scan: a matrix batch is thousands of ids,
    // and the allocation is proportional to the id count like everything else on
    // this path.
    let mut seen = std::collections::HashSet::new();
    seen.try_reserve(ids.len())
        .map_err(|_| "column batch id set allocation failed".to_string())?;
    for id in ids {
        validate_column_registry_action(columns.contains_key(id), ColumnRegistryAction::Register)
            .map_err(|reason| format!("column '{id}' {reason}"))?;
        if !seen.insert(id.as_str()) {
            return Err(format!("column '{id}' appears twice in one batch"));
        }
    }
    Ok(())
}

/// Registry metadata for a whole batch, prepared into fresh maps.
///
/// Same prepare-then-commit shape as [`prepare_demo_column_metadata`], for the
/// same reason: nothing installs until the renderer's own atomic insert has
/// succeeded. Each id gets its own revision, so a host watching revisions sees
/// one bump per column rather than one for the batch.
#[cfg(any(target_arch = "wasm32", test))]
fn prepare_batch_column_metadata(
    columns: &std::collections::HashMap<String, usize>,
    revisions: &std::collections::HashMap<String, u64>,
    next_revision: u64,
    ids: &[String],
    len: usize,
) -> Result<PreparedColumnMetadata, &'static str> {
    use std::collections::HashMap;

    let column_capacity = columns
        .len()
        .checked_add(ids.len())
        .ok_or("column metadata capacity exhausted")?;
    let mut future_columns = HashMap::new();
    future_columns
        .try_reserve(column_capacity)
        .map_err(|_| "column metadata allocation failed")?;
    future_columns.extend(columns.iter().map(|(id, value)| (id.clone(), *value)));

    let revision_capacity = revisions
        .len()
        .checked_add(ids.len())
        .ok_or("column revision metadata capacity exhausted")?;
    let mut future_revisions = HashMap::new();
    future_revisions
        .try_reserve(revision_capacity)
        .map_err(|_| "column revision metadata allocation failed")?;
    future_revisions.extend(revisions.iter().map(|(id, value)| (id.clone(), *value)));

    let mut successor = next_revision;
    for id in ids {
        let (revision, next) = checked_column_revision(successor)?;
        future_columns.insert(id.clone(), len);
        future_revisions.insert(id.clone(), revision);
        successor = next;
    }
    Ok(PreparedColumnMetadata {
        columns: future_columns,
        revisions: future_revisions,
        next_revision: successor,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn prepare_column_metadata(
    columns: &std::collections::HashMap<String, usize>,
    revisions: &std::collections::HashMap<String, u64>,
    next_revision: u64,
    id: &str,
    len: usize,
) -> Result<PreparedColumnMetadata, &'static str> {
    use std::collections::HashMap;

    let (revision, successor) = checked_column_revision(next_revision)?;
    let capacity = columns
        .len()
        .checked_add(1)
        .ok_or("column metadata capacity exhausted")?;
    let mut future_columns = HashMap::new();
    future_columns
        .try_reserve(capacity)
        .map_err(|_| "column metadata allocation failed")?;
    future_columns.extend(columns.iter().map(|(id, value)| (id.clone(), *value)));
    future_columns.insert(id.to_string(), len);

    let capacity = revisions
        .len()
        .checked_add(1)
        .ok_or("column revision metadata capacity exhausted")?;
    let mut future_revisions = HashMap::new();
    future_revisions
        .try_reserve(capacity)
        .map_err(|_| "column revision metadata allocation failed")?;
    future_revisions.extend(revisions.iter().map(|(id, value)| (id.clone(), *value)));
    future_revisions.insert(id.to_string(), revision);

    Ok(PreparedColumnMetadata {
        columns: future_columns,
        revisions: future_revisions,
        next_revision: successor,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn prepare_demo_column_metadata(
    columns: &std::collections::HashMap<String, usize>,
    revisions: &std::collections::HashMap<String, u64>,
    next_revision: u64,
    lengths: [usize; 4],
) -> Result<PreparedColumnMetadata, &'static str> {
    use std::collections::HashMap;

    let additional_columns = DEMO_COLUMN_IDS
        .iter()
        .filter(|id| !columns.contains_key(**id))
        .count();
    let column_capacity = columns
        .len()
        .checked_add(additional_columns)
        .ok_or("column metadata capacity exhausted")?;
    let mut future_columns = HashMap::new();
    future_columns
        .try_reserve(column_capacity)
        .map_err(|_| "column metadata allocation failed")?;
    future_columns.extend(columns.iter().map(|(id, value)| (id.clone(), *value)));

    let additional_revisions = DEMO_COLUMN_IDS
        .iter()
        .filter(|id| !revisions.contains_key(**id))
        .count();
    let revision_capacity = revisions
        .len()
        .checked_add(additional_revisions)
        .ok_or("column revision metadata capacity exhausted")?;
    let mut future_revisions = HashMap::new();
    future_revisions
        .try_reserve(revision_capacity)
        .map_err(|_| "column revision metadata allocation failed")?;
    future_revisions.extend(revisions.iter().map(|(id, value)| (id.clone(), *value)));

    let mut successor = next_revision;
    for (id, len) in DEMO_COLUMN_IDS.into_iter().zip(lengths) {
        let (revision, next) = checked_column_revision(successor)?;
        future_columns.insert(id.to_string(), len);
        future_revisions.insert(id.to_string(), revision);
        successor = next;
    }
    Ok(PreparedColumnMetadata {
        columns: future_columns,
        revisions: future_revisions,
        next_revision: successor,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
struct PreparedDemoDeclarations {
    config: renderer::Config,
    series: Vec<renderer::SeriesConfig>,
    labels: Vec<Option<String>>,
    color_seq: usize,
}

#[cfg(any(target_arch = "wasm32", test))]
fn prepare_demo_declarations(
    current_config: &renderer::Config,
    current_series: &[renderer::SeriesConfig],
    current_labels: &[Option<String>],
    cycle: renderer::ColorCycle,
    color_seq: usize,
) -> Result<PreparedDemoDeclarations, String> {
    use renderer::line::LineStylePreset;
    use renderer::{DataLineStyleConfig, DataRenderType, SeriesConfig};

    let mut config = current_config.clone();
    let mut series = Vec::new();
    series
        .try_reserve(current_series.len().saturating_add(2))
        .map_err(|error| error.to_string())?;
    series.extend(current_series.iter().cloned());
    let mut labels = Vec::new();
    labels
        .try_reserve(current_labels.len().saturating_add(2))
        .map_err(|error| error.to_string())?;
    labels.extend(current_labels.iter().cloned());
    let mut next_color_seq = color_seq;

    for (series_id, x_column, y_column, label) in [
        ("sine", "demo_x", "demo_sin", "sin(x)"),
        ("rc", "demo_t", "demo_rc", "RC charge"),
    ] {
        let existing = series.iter().position(|item| item.series_id == series_id);
        let color = match existing {
            // Exhaustive so a new render type has to declare where its colour
            // comes from. A wildcard here would hand a bar or field series the
            // cycle colour while its own configured colour sat unread.
            Some(index) => match &series[index].render_type {
                DataRenderType::Line { line } => line.line_color,
                DataRenderType::Scatter { .. }
                | DataRenderType::ScatterLine { .. }
                | DataRenderType::ScatterErrorbarX { .. }
                | DataRenderType::ScatterErrorbarY { .. }
                | DataRenderType::ScatterErrorbarXY { .. }
                | DataRenderType::LineScatterErrorbarX { .. }
                | DataRenderType::LineScatterErrorbarY { .. }
                | DataRenderType::LineScatterErrorbarXY { .. } => cycle.color(index),
                DataRenderType::Histogram { bar } => bar.fill_color,
                DataRenderType::Contour { contour, .. }
                | DataRenderType::HeatmapContour { contour, .. } => contour.line.line_color,
                // A heatmap's colour is the colourbar's colormap, not a swatch
                // this demo could reuse.
                DataRenderType::Heatmap { .. } => cycle.color(index),
            },
            None => cycle.color(next_color_seq),
        };
        if existing.is_none() {
            next_color_seq = next_color_seq
                .checked_add(1)
                .ok_or_else(|| "series color sequence exhausted".to_string())?;
        }
        let rich_label = renderer::text::RichText {
            segments: renderer::text::rich_segments_from_text(label),
            color: config.legend.content.color,
            font_size: config.legend.content.font_size,
            font: config.legend.content.font.clone(),
        };
        let item = SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: Some(rich_label.clone()),
            x_column: x_column.to_string(),
            y_column: y_column.to_string(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: color,
                    line_width: 2.0,
                },
            },
        };
        if let Some(index) = existing {
            series[index] = item.clone();
            labels[index] = Some(label.to_string());
            renderer::config::set_legend_entry_label(
                &mut config.legend.content,
                index,
                renderer::config::series_symbol_segments(&item),
                rich_label.segments,
            );
        } else {
            series.push(item.clone());
            labels.push(Some(label.to_string()));
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(&item),
                rich_label.segments,
            );
        }
        config.legend.visible = true;
    }
    config.chart_title.text.segments = renderer::text::rich_segments_from_text("figgy");
    config.bottom_x.title_option.text.segments = renderer::text::rich_segments_from_text("x");
    config.left_y.title_option.text.segments = renderer::text::rich_segments_from_text("y");

    Ok(PreparedDemoDeclarations {
        config,
        series,
        labels,
        color_seq: next_color_seq,
    })
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, rc::Rc};

    use renderer::data_config::{
        ContourConfig, ErrorRef, FieldFillConfig, FillMode, GridLayout, MatrixOrientation,
        MatrixRef, Shading,
    };
    use renderer::{
        Color, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
        DataScatterStyleConfig, GpuFieldExtentMode, GpuSeriesExtentMode, GpuSeriesFitMode,
        ScatterShape, SeriesConfig,
    };

    use super::{
        ColumnRegistryAction, DEMO_COLUMN_IDS, ErrorRefKind, FrameDecision,
        active_series_extent_requests, checked_column_revision, consume_successful_frame,
        frame_decision, picked_data_json_string, picked_point_json_string,
        prepare_batch_column_metadata, prepare_column_metadata, prepare_demo_column_metadata,
        prepare_demo_declarations, retain_valid_series_extent_jobs, select_series_extent_job_map,
        series_extent_key_matches_config, series_extent_mode, series_extent_needs_submission,
        series_extent_request_from, validate_column_batch, validate_column_data_len,
        validate_column_registry_action,
    };
    use crate::scalar_job::{SeriesExtentJob, SeriesExtentStatus};

    fn scatter_style() -> DataScatterStyleConfig {
        DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::Circle,
            point_size: 4.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
        }
    }

    fn errorbar_style() -> DataErrorBarStyleConfig {
        DataErrorBarStyleConfig {
            error_bar_color: Color::BLACK,
            error_bar_width: 1.0,
            error_bar_cap_size: 2.0,
            cap_width: 1.0,
            error_bar_style_table: None,
            error_bar_style_index_column: None,
            error_bar_style_overrides: None,
        }
    }

    fn line_style() -> DataLineStyleConfig {
        DataLineStyleConfig {
            line_style: renderer::line::LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 1.0,
        }
    }

    fn series_config(series_id: &str, render_type: DataRenderType) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: None,
            x_column: "x".to_string(),
            y_column: "y".to_string(),
            render_type,
        }
    }

    fn line_series(series_id: &str, x: &str, y: &str) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_string(),
            source_id: None,
            label: None,
            x_column: x.to_string(),
            y_column: y.to_string(),
            render_type: DataRenderType::Line { line: line_style() },
        }
    }

    #[test]
    fn frame_decision_covers_every_dirty_source_without_surface_work_when_clean() {
        for bits in 0u8..32 {
            let renderer_dirty = bits & 1 != 0;
            let raster_dirty = bits & 2 != 0;
            let view_dirty = bits & 4 != 0;
            let redraw_pending = bits & 8 != 0;
            let maintenance_pending = bits & 16 != 0;
            let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
            let expected = if !visual_pending {
                if maintenance_pending {
                    FrameDecision::MaintenanceOnly
                } else {
                    FrameDecision::Clean
                }
            } else {
                FrameDecision::Draw {
                    refresh_raster: raster_dirty || view_dirty,
                }
            };

            assert_eq!(
                frame_decision(
                    renderer_dirty,
                    raster_dirty,
                    view_dirty,
                    redraw_pending,
                    maintenance_pending,
                ),
                expected,
                "dirty-state mask {bits:05b}"
            );
        }
    }

    #[test]
    fn initial_state_requires_the_first_draw() {
        assert_eq!(
            frame_decision(true, true, false, true, false),
            FrameDecision::Draw {
                refresh_raster: true
            }
        );
    }

    #[test]
    fn successful_frame_consumes_visual_state_but_failed_frame_preserves_it() {
        let mut successful_view_dirty = true;
        let mut successful_redraw_pending = true;
        consume_successful_frame(&mut successful_view_dirty, &mut successful_redraw_pending);
        assert!(!successful_view_dirty);
        assert!(!successful_redraw_pending);

        let failed_view_dirty = true;
        let failed_redraw_pending = true;
        assert!(failed_view_dirty);
        assert!(failed_redraw_pending);
    }

    #[test]
    fn display_panel_uniformly_scales_document() {
        let (scale, panel) = renderer::fit_display_panel((1000, 800), (2000, 1600));
        assert!((scale - 2.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 2000, 1600)
        );
    }

    #[test]
    fn display_panel_letterboxes_aspect_ratio_changes() {
        let (scale, panel) = renderer::fit_display_panel((1000, 800), (1600, 800));
        assert!((scale - 1.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (300, 0, 1000, 800)
        );
    }

    #[test]
    fn display_config_scales_without_mutating_logical_document() {
        let mut config = renderer::default::default_config();
        config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        });
        let original = config.clone();

        let (display, panel, scale) = renderer::display_config_for_surface(&config, (500, 400));

        assert!((scale - 0.5).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 500, 400)
        );
        assert_eq!(config.chart_area, original.chart_area);
        assert_eq!(
            config.bottom_x.label_style.font_size,
            original.bottom_x.label_style.font_size
        );
        assert_eq!(display.chart_area.0.width, 500);
        assert!((display.bottom_x.label_style.font_size - 9.0).abs() < 1e-6);
    }

    #[test]
    fn picked_point_json_matches_public_contract() {
        let picked = renderer::PickedPoint {
            source_id: None,
            series_id: "series-a".into(),
            point_index: 7,
            distance_px: 4.0,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_point_json_string(&picked).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "source_id": null,
                "series_id": "series-a",
                "point_index": 7,
                "distance_px": 4.0,
            })
        );
    }

    #[test]
    fn picked_data_json_is_tagged_and_keeps_canonical_indices() {
        let picked = renderer::PickedData::ContourLevel {
            source_id: Some("source-grid".into()),
            series_id: "contour-a".into(),
            level_index: 4,
            x_index: 2,
            y_index: 3,
            distance_px: 0.75,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&picked).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "kind": "contour_level",
                "source_id": "source-grid",
                "series_id": "contour-a",
                "level_index": 4,
                "x_index": 2,
                "y_index": 3,
                "distance_px": 0.75,
            })
        );
    }

    #[test]
    fn column_revision_successor_rejects_overflow() {
        assert_eq!(checked_column_revision(41), Ok((41, 42)));
        assert_eq!(
            checked_column_revision(u64::MAX),
            Err("column revision counter exhausted")
        );
    }

    #[test]
    fn all_render_types_normalize_to_their_drawable_extent_domain() {
        let x_error = ErrorRef::Symmetric {
            column: "x_err".to_string(),
        };
        let y_error = ErrorRef::Symmetric {
            column: "y_err".to_string(),
        };
        let cases = [
            (
                DataRenderType::Line { line: line_style() },
                GpuSeriesExtentMode::Line,
            ),
            (
                DataRenderType::Scatter {
                    scatter: scatter_style(),
                },
                GpuSeriesExtentMode::Points,
            ),
            (
                DataRenderType::ScatterLine {
                    scatter: scatter_style(),
                    line: line_style(),
                },
                GpuSeriesExtentMode::Points,
            ),
            (
                DataRenderType::ScatterErrorbarX {
                    scatter: scatter_style(),
                    err_x: x_error.clone(),
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsX,
            ),
            (
                DataRenderType::ScatterErrorbarY {
                    scatter: scatter_style(),
                    err_y: y_error.clone(),
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsY,
            ),
            (
                DataRenderType::ScatterErrorbarXY {
                    scatter: scatter_style(),
                    err_x: x_error.clone(),
                    err_y: y_error.clone(),
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsXY,
            ),
            (
                DataRenderType::LineScatterErrorbarX {
                    scatter: scatter_style(),
                    line: line_style(),
                    err_x: x_error.clone(),
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsX,
            ),
            (
                DataRenderType::LineScatterErrorbarY {
                    scatter: scatter_style(),
                    line: line_style(),
                    err_y: y_error.clone(),
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsY,
            ),
            (
                DataRenderType::LineScatterErrorbarXY {
                    scatter: scatter_style(),
                    line: line_style(),
                    err_x: x_error,
                    err_y: y_error,
                    err_style: errorbar_style(),
                },
                GpuSeriesExtentMode::PointsXY,
            ),
        ];

        for (render_type, expected) in cases {
            assert_eq!(
                series_extent_mode(&render_type),
                Some(GpuSeriesFitMode::Paired(expected))
            );
        }

        let matrix = |grid_layout| MatrixRef {
            columns: vec!["z0".into(), "z1".into()],
            orientation: MatrixOrientation::ColumnsAreX,
            grid_layout,
        };
        let fill = |shading| FieldFillConfig {
            mode: FillMode::Continuous,
            shading,
            opacity: 1.0,
        };
        let contour = || ContourConfig {
            levels: vec![0.5],
            line: line_style(),
            per_level_color: None,
            labels: None,
        };
        let field_cases = [
            (
                DataRenderType::Heatmap {
                    matrix: matrix(GridLayout::Edges),
                    fill: fill(Shading::Flat),
                },
                GpuFieldExtentMode::EdgesCells,
            ),
            (
                DataRenderType::Heatmap {
                    matrix: matrix(GridLayout::Edges),
                    fill: fill(Shading::Interpolated),
                },
                GpuFieldExtentMode::EdgesSamples,
            ),
            (
                DataRenderType::Contour {
                    matrix: matrix(GridLayout::Centers),
                    contour: contour(),
                },
                GpuFieldExtentMode::CentersSamples,
            ),
            (
                DataRenderType::HeatmapContour {
                    matrix: matrix(GridLayout::Centers),
                    fill: fill(Shading::Flat),
                    contour: contour(),
                },
                GpuFieldExtentMode::CentersCells,
            ),
            (
                DataRenderType::HeatmapContour {
                    matrix: matrix(GridLayout::Centers),
                    fill: fill(Shading::Interpolated),
                    contour: contour(),
                },
                GpuFieldExtentMode::CentersSamples,
            ),
        ];
        for (render_type, expected) in field_cases {
            assert_eq!(
                series_extent_mode(&render_type),
                Some(GpuSeriesFitMode::Field(expected))
            );
        }
    }

    #[test]
    fn series_extent_key_is_role_explicit_and_excludes_unrelated_columns() {
        let revisions = HashMap::from([
            ("x".to_string(), 1),
            ("y".to_string(), 2),
            ("err".to_string(), 3),
            ("unrelated".to_string(), 4),
        ]);
        let plain = series_config(
            "plain",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );
        let plain_request = series_extent_request_from(&revisions, &plain).unwrap();
        assert_eq!(
            plain_request.key.mode,
            GpuSeriesFitMode::Paired(GpuSeriesExtentMode::Points)
        );
        assert!(plain_request.key.x_error.is_none());
        assert!(plain_request.key.y_error.is_none());
        assert!(!format!("{:?}", plain_request.key).contains("unrelated"));
        let mut unrelated_changed = revisions.clone();
        unrelated_changed.insert("unrelated".to_string(), 99);
        assert_eq!(
            series_extent_request_from(&unrelated_changed, &plain)
                .unwrap()
                .key,
            plain_request.key
        );

        let symmetric = series_config(
            "symmetric",
            DataRenderType::ScatterErrorbarX {
                scatter: scatter_style(),
                err_x: ErrorRef::Symmetric {
                    column: "err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        let asymmetric = series_config(
            "asymmetric",
            DataRenderType::ScatterErrorbarX {
                scatter: scatter_style(),
                err_x: ErrorRef::Asymmetric {
                    lower: "err".to_string(),
                    upper: "err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        let symmetric_key = series_extent_request_from(&revisions, &symmetric)
            .unwrap()
            .key;
        let asymmetric_key = series_extent_request_from(&revisions, &asymmetric)
            .unwrap()
            .key;
        assert_eq!(symmetric_key.x_error.as_ref().unwrap().lower.id, "err");
        assert_eq!(symmetric_key.x_error.as_ref().unwrap().upper.id, "err");
        assert!(symmetric_key.y_error.is_none());
        assert_eq!(
            symmetric_key.x_error.as_ref().unwrap().kind,
            ErrorRefKind::Symmetric
        );
        assert_eq!(
            asymmetric_key.x_error.as_ref().unwrap().kind,
            ErrorRefKind::Asymmetric
        );
        assert_ne!(symmetric_key, asymmetric_key);
    }

    #[test]
    fn series_extent_key_keeps_mode_and_every_named_role_distinct() {
        let revisions = HashMap::from([
            ("x".to_string(), 7),
            ("y".to_string(), 7),
            ("err".to_string(), 7),
            ("lo".to_string(), 7),
            ("hi".to_string(), 7),
        ]);
        let key_for = |cfg: &SeriesConfig| series_extent_request_from(&revisions, cfg).unwrap().key;

        let x_only = series_config(
            "x-only",
            DataRenderType::ScatterErrorbarX {
                scatter: scatter_style(),
                err_x: ErrorRef::Symmetric {
                    column: "err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        let y_only = series_config(
            "y-only",
            DataRenderType::ScatterErrorbarY {
                scatter: scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        assert_ne!(key_for(&x_only), key_for(&y_only));

        let points = series_config(
            "points",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );
        let mut swapped = points.clone();
        swapped.x_column = "y".to_string();
        swapped.y_column = "x".to_string();
        assert_ne!(key_for(&points), key_for(&swapped));

        let asymmetric = |lower: &str, upper: &str| {
            series_config(
                "asymmetric",
                DataRenderType::ScatterErrorbarX {
                    scatter: scatter_style(),
                    err_x: ErrorRef::Asymmetric {
                        lower: lower.to_string(),
                        upper: upper.to_string(),
                    },
                    err_style: errorbar_style(),
                },
            )
        };
        assert_ne!(
            key_for(&asymmetric("lo", "hi")),
            key_for(&asymmetric("hi", "lo"))
        );

        let line = series_config("line", DataRenderType::Line { line: line_style() });
        assert_ne!(key_for(&line), key_for(&points));

        let field = series_config(
            "field",
            DataRenderType::Heatmap {
                matrix: MatrixRef {
                    columns: vec!["z0".into(), "z1".into()],
                    orientation: MatrixOrientation::ColumnsAreX,
                    grid_layout: GridLayout::Centers,
                },
                fill: FieldFillConfig {
                    mode: FillMode::Continuous,
                    shading: Shading::Flat,
                    opacity: 1.0,
                },
            },
        );
        let mut transposed = field.clone();
        let DataRenderType::Heatmap { matrix, .. } = &mut transposed.render_type else {
            unreachable!()
        };
        matrix.orientation = MatrixOrientation::ColumnsAreY;
        let field_revisions = HashMap::from([
            ("x".to_string(), 7),
            ("y".to_string(), 7),
            ("z0".to_string(), 7),
            ("z1".to_string(), 7),
        ]);
        let field_key = |cfg: &SeriesConfig| {
            series_extent_request_from(&field_revisions, cfg)
                .unwrap()
                .key
        };
        assert_ne!(field_key(&field), field_key(&transposed));
        let mut changed_matrix_revision = field_revisions.clone();
        changed_matrix_revision.insert("z1".to_string(), 8);
        assert_ne!(
            field_key(&field),
            series_extent_request_from(&changed_matrix_revision, &field)
                .unwrap()
                .key
        );
        let mut reordered = field.clone();
        let DataRenderType::Heatmap { matrix, .. } = &mut reordered.render_type else {
            unreachable!()
        };
        matrix.columns.reverse();
        assert_ne!(field_key(&field), field_key(&reordered));
    }

    #[test]
    fn active_series_extent_requests_fail_closed_on_missing_revision() {
        let revisions = HashMap::from([("x".to_string(), 1)]);
        let series = series_config(
            "missing-y",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );

        let error = active_series_extent_requests(&[series], &revisions).unwrap_err();
        assert_eq!(
            error.to_string(),
            "series references a column without a live revision"
        );
    }

    #[test]
    fn active_series_extent_requests_leave_histograms_to_metadata_fit() {
        let revisions = HashMap::from([("x".to_string(), 1), ("y".to_string(), 2)]);
        let series = series_config(
            "histogram",
            DataRenderType::Histogram {
                bar: renderer::data_config::DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 1.0,
                    orientation: renderer::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
        );

        assert!(
            active_series_extent_requests(&[series], &revisions)
                .unwrap()
                .is_empty(),
            "non-paired render types are fitted from upload metadata"
        );
    }

    #[test]
    fn histogram_fit_columns_follow_orientation_roles() {
        let mut series = series_config(
            "histogram",
            DataRenderType::Histogram {
                bar: renderer::data_config::DataBarStyleConfig {
                    fill_color: Color::BLACK,
                    border_color: Color::BLACK,
                    border_width: 1.0,
                    baseline: 0.0,
                    gap_px: 1.0,
                    width_ratio: 1.0,
                    orientation: renderer::data_config::BarOrientation::Vertical,
                    bar_style_overrides: None,
                },
            },
        );
        series.x_column = "edges".to_string();
        series.y_column = "counts".to_string();
        assert_eq!(
            crate::histogram_fit_columns(&series, &renderer::data_config::BarOrientation::Vertical,),
            ("edges", "counts"),
        );

        series.x_column = "counts".to_string();
        series.y_column = "edges".to_string();
        assert_eq!(
            crate::histogram_fit_columns(
                &series,
                &renderer::data_config::BarOrientation::Horizontal,
            ),
            ("edges", "counts"),
        );
    }

    #[test]
    fn future_series_extent_map_reuses_only_identical_revision_keys() {
        let old_revisions = HashMap::from([
            ("stable-x".to_string(), 1),
            ("stable-y".to_string(), 2),
            ("changed-x".to_string(), 3),
            ("changed-y".to_string(), 4),
        ]);
        let mut future_revisions = old_revisions.clone();
        future_revisions.insert("changed-x".to_string(), 5);
        let mut stable = series_config(
            "stable",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );
        stable.x_column = "stable-x".to_string();
        stable.y_column = "stable-y".to_string();
        let mut changed = stable.clone();
        changed.series_id = "changed".to_string();
        changed.x_column = "changed-x".to_string();
        changed.y_column = "changed-y".to_string();
        let mut stable_alias = stable.clone();
        stable_alias.series_id = "stable-alias".to_string();

        let stable_request = series_extent_request_from(&future_revisions, &stable).unwrap();
        let old_changed_request = series_extent_request_from(&old_revisions, &changed).unwrap();
        let future_changed_request =
            series_extent_request_from(&future_revisions, &changed).unwrap();
        let stable_job = SeriesExtentJob::pending();
        let old_changed_job = SeriesExtentJob::pending();
        let existing = HashMap::from([
            (stable_request.key.clone(), Rc::clone(&stable_job)),
            (old_changed_request.key, Rc::clone(&old_changed_job)),
        ]);

        let active =
            active_series_extent_requests(&[stable, stable_alias, changed], &future_revisions)
                .unwrap();
        assert_eq!(active.len(), 2);
        let (selected, pending) = select_series_extent_job_map(active, &existing, false).unwrap();

        assert!(Rc::ptr_eq(
            selected.get(&stable_request.key).unwrap(),
            &stable_job
        ));
        assert!(!Rc::ptr_eq(
            selected.get(&future_changed_request.key).unwrap(),
            &old_changed_job
        ));
        assert_eq!(pending, vec![future_changed_request]);
    }

    #[test]
    fn series_extent_transitions_cover_add_set_remove_and_retry() {
        let revisions = HashMap::from([
            ("x".to_string(), 1),
            ("y".to_string(), 2),
            ("x2".to_string(), 3),
            ("y2".to_string(), 4),
        ]);
        let a = series_config(
            "a",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );
        let mut b = a.clone();
        b.series_id = "b".to_string();
        b.x_column = "x2".to_string();
        b.y_column = "y2".to_string();
        let a_request = series_extent_request_from(&revisions, &a).unwrap();
        let b_request = series_extent_request_from(&revisions, &b).unwrap();
        let a_job = SeriesExtentJob::pending();
        a_job.complete_success(None);
        let existing = HashMap::from([(a_request.key.clone(), Rc::clone(&a_job))]);

        let add_requests =
            active_series_extent_requests(&[a.clone(), b.clone()], &revisions).unwrap();
        let (after_add, add_pending) =
            select_series_extent_job_map(add_requests, &existing, false).unwrap();
        assert!(Rc::ptr_eq(after_add.get(&a_request.key).unwrap(), &a_job));
        assert_eq!(add_pending, vec![b_request.clone()]);
        assert_eq!(
            existing.len(),
            1,
            "future map must not publish into old state"
        );

        let mut with_unrelated_column = revisions.clone();
        with_unrelated_column.insert("unrelated".to_string(), 99);
        let unrelated_requests =
            active_series_extent_requests(std::slice::from_ref(&a), &with_unrelated_column)
                .unwrap();
        let (after_unrelated_column, unrelated_pending) =
            select_series_extent_job_map(unrelated_requests, &existing, false).unwrap();
        assert!(unrelated_pending.is_empty());
        assert!(Rc::ptr_eq(
            after_unrelated_column.get(&a_request.key).unwrap(),
            &a_job
        ));

        let line_a = series_config("a", DataRenderType::Line { line: line_style() });
        let line_request = series_extent_request_from(&revisions, &line_a).unwrap();
        let (after_set, set_pending) = select_series_extent_job_map(
            active_series_extent_requests(&[line_a], &revisions).unwrap(),
            &existing,
            false,
        )
        .unwrap();
        assert_eq!(set_pending, vec![line_request.clone()]);
        assert!(!after_set.contains_key(&a_request.key));

        let mut after_remove = after_add;
        after_remove.retain(|key, _| series_extent_key_matches_config(key, &a, &revisions));
        assert_eq!(after_remove.len(), 1);
        assert!(after_remove.contains_key(&a_request.key));
        assert!(!after_remove.contains_key(&b_request.key));

        let retry_job = SeriesExtentJob::pending();
        retry_job.complete_retryable_failure("readback failed".to_string());
        let retry_existing = HashMap::from([(a_request.key.clone(), Rc::clone(&retry_job))]);
        let retry_requests =
            active_series_extent_requests(std::slice::from_ref(&a), &revisions).unwrap();
        let (without_retry, pending) =
            select_series_extent_job_map(retry_requests.clone(), &retry_existing, false).unwrap();
        assert!(pending.is_empty());
        assert!(Rc::ptr_eq(
            without_retry.get(&a_request.key).unwrap(),
            &retry_job
        ));
        let (with_retry, pending) =
            select_series_extent_job_map(retry_requests, &retry_existing, true).unwrap();
        assert_eq!(pending, vec![a_request.clone()]);
        assert!(!Rc::ptr_eq(
            with_retry.get(&a_request.key).unwrap(),
            &retry_job
        ));
        assert_eq!(retry_existing.len(), 1);
    }

    #[test]
    fn series_extent_retry_policy_replaces_only_retryable_failures() {
        assert!(series_extent_needs_submission(None, false));
        assert!(!series_extent_needs_submission(
            Some(SeriesExtentStatus::RetryableFailed),
            false
        ));
        assert!(series_extent_needs_submission(
            Some(SeriesExtentStatus::RetryableFailed),
            true
        ));
        assert!(!series_extent_needs_submission(
            Some(SeriesExtentStatus::Pending),
            true
        ));
        assert!(!series_extent_needs_submission(
            Some(SeriesExtentStatus::Succeeded),
            true
        ));
        assert!(!series_extent_needs_submission(
            Some(SeriesExtentStatus::TerminalFailed),
            true
        ));
    }

    #[test]
    fn metadata_preflight_does_not_publish_before_commit() {
        let columns = HashMap::from([("x".to_string(), 2)]);
        let revisions = HashMap::from([("x".to_string(), 7)]);
        let original_columns = columns.clone();
        let original_revisions = revisions.clone();

        let prepared = prepare_column_metadata(&columns, &revisions, 8, "x", 3).unwrap();

        assert_eq!(columns, original_columns);
        assert_eq!(revisions, original_revisions);
        assert_eq!(prepared.columns.get("x"), Some(&3));
        assert_eq!(prepared.revisions.get("x"), Some(&8));
        assert_eq!(prepared.next_revision, 9);
    }

    #[test]
    fn same_length_update_still_issues_a_new_revision() {
        let columns = HashMap::from([("x".to_string(), 2)]);
        let revisions = HashMap::from([("x".to_string(), 7)]);

        let prepared = prepare_column_metadata(&columns, &revisions, 8, "x", 2).unwrap();

        assert_eq!(prepared.columns.get("x"), Some(&2));
        assert_eq!(prepared.revisions.get("x"), Some(&8));
        assert_eq!(prepared.next_revision, 9);
    }

    #[test]
    fn demo_metadata_issues_four_revisions_without_partial_publication() {
        let columns = HashMap::from([("stable".to_string(), 7), ("demo_x".to_string(), 3)]);
        let revisions = HashMap::from([("stable".to_string(), 10), ("demo_x".to_string(), 11)]);
        let original_columns = columns.clone();
        let original_revisions = revisions.clone();
        let prepared = prepare_demo_column_metadata(&columns, &revisions, 20, [512; 4]).unwrap();

        assert_eq!(columns, original_columns);
        assert_eq!(revisions, original_revisions);
        assert_eq!(prepared.columns.get("stable"), Some(&7));
        assert_eq!(prepared.revisions.get("stable"), Some(&10));
        for (index, id) in DEMO_COLUMN_IDS.into_iter().enumerate() {
            assert_eq!(prepared.columns.get(id), Some(&512));
            assert_eq!(prepared.revisions.get(id), Some(&(20 + index as u64)));
        }
        assert_eq!(prepared.next_revision, 24);

        let error = match prepare_demo_column_metadata(&columns, &revisions, u64::MAX - 3, [512; 4])
        {
            Ok(_) => panic!("fourth demo revision must reject overflow"),
            Err(error) => error,
        };
        assert_eq!(error, "column revision counter exhausted");
        assert_eq!(columns, original_columns);
        assert_eq!(revisions, original_revisions);
    }

    #[test]
    fn batch_metadata_issues_one_revision_per_column_without_partial_publication() {
        let columns = HashMap::from([("stable".to_string(), 7)]);
        let revisions = HashMap::from([("stable".to_string(), 10)]);
        let original_columns = columns.clone();
        let original_revisions = revisions.clone();
        let ids = ["g0".to_string(), "g1".to_string(), "g2".to_string()];

        let prepared = prepare_batch_column_metadata(&columns, &revisions, 20, &ids, 64).unwrap();
        // The caller's maps are untouched until it installs the prepared ones.
        assert_eq!(columns, original_columns);
        assert_eq!(revisions, original_revisions);
        assert_eq!(prepared.columns.get("stable"), Some(&7));
        assert_eq!(prepared.revisions.get("stable"), Some(&10));
        for (index, id) in ids.iter().enumerate() {
            assert_eq!(prepared.columns.get(id), Some(&64));
            assert_eq!(prepared.revisions.get(id), Some(&(20 + index as u64)));
        }
        assert_eq!(prepared.next_revision, 23);

        // Overflow on the third id rejects the whole batch, not the first two.
        let error =
            match prepare_batch_column_metadata(&columns, &revisions, u64::MAX - 2, &ids, 64) {
                Ok(_) => panic!("a batch revision past the counter must be rejected"),
                Err(error) => error,
            };
        assert_eq!(error, "column revision counter exhausted");
        assert_eq!(columns, original_columns);
        assert_eq!(revisions, original_revisions);
    }

    #[test]
    fn a_column_batch_is_validated_before_anything_is_uploaded() {
        let columns = HashMap::from([("taken".to_string(), 4)]);
        let ids = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();

        // The shape that works: 3 ids x 2 values = 6.
        assert!(validate_column_batch(&columns, &ids(&["a", "b", "c"]), 2, 6).is_ok());

        assert_eq!(
            validate_column_batch(&columns, &[], 2, 0).unwrap_err(),
            "a column batch needs at least one id"
        );
        assert_eq!(
            validate_column_batch(&columns, &ids(&["a"]), 0, 0).unwrap_err(),
            "values_per_column data must not be empty"
        );
        // A flat buffer that does not divide into the declared shape is not a
        // length mismatch to draw through — it cannot be cut into columns at all.
        assert!(
            validate_column_batch(&columns, &ids(&["a", "b"]), 3, 5)
                .unwrap_err()
                .contains("5 values")
        );
        assert_eq!(
            validate_column_batch(&columns, &ids(&["a", "taken"]), 2, 4).unwrap_err(),
            "column 'taken' is already registered"
        );
        assert_eq!(
            validate_column_batch(&columns, &ids(&["a", "a"]), 2, 4).unwrap_err(),
            "column 'a' appears twice in one batch"
        );
        assert!(validate_column_batch(&columns, &ids(&["__zero"]), 2, 2).is_ok());
    }

    #[test]
    fn demo_declarations_are_complete_and_stable_on_repeat() {
        let initial_config = renderer::default::default_config();
        let first =
            prepare_demo_declarations(&initial_config, &[], &[], renderer::ColorCycle::Classic, 0)
                .unwrap();
        assert_eq!(
            first
                .series
                .iter()
                .map(|item| item.series_id.as_str())
                .collect::<Vec<_>>(),
            vec!["sine", "rc"]
        );
        assert_eq!(
            first.labels,
            vec![Some("sin(x)".into()), Some("RC charge".into())]
        );
        assert_eq!(first.color_seq, 2);
        assert_eq!(
            first.config.chart_title.text.segments,
            renderer::text::rich_segments_from_text("figgy")
        );
        assert_eq!(
            first.config.bottom_x.title_option.text.segments,
            renderer::text::rich_segments_from_text("x")
        );
        assert_eq!(
            first.config.left_y.title_option.text.segments,
            renderer::text::rich_segments_from_text("y")
        );

        let repeated = prepare_demo_declarations(
            &first.config,
            &first.series,
            &first.labels,
            renderer::ColorCycle::Classic,
            first.color_seq,
        )
        .unwrap();
        assert_eq!(repeated.config, first.config);
        assert_eq!(repeated.series, first.series);
        assert_eq!(repeated.labels, first.labels);
        assert_eq!(repeated.color_seq, first.color_seq);
    }

    #[test]
    fn demo_extent_preparation_retains_stable_jobs_and_defers_changed_submits() {
        let stable = line_series("stable", "stable-x", "stable-y");
        let demo = line_series("demo", "demo_x", "demo_sin");
        let old_revisions = HashMap::from([
            ("stable-x".to_string(), 1),
            ("stable-y".to_string(), 1),
            ("demo_x".to_string(), 1),
            ("demo_sin".to_string(), 1),
        ]);
        let stable_request = series_extent_request_from(&old_revisions, &stable).unwrap();
        let old_demo_request = series_extent_request_from(&old_revisions, &demo).unwrap();
        let stable_job = SeriesExtentJob::pending();
        let demo_job = SeriesExtentJob::pending();
        let existing = HashMap::from([
            (stable_request.key.clone(), Rc::clone(&stable_job)),
            (old_demo_request.key, demo_job),
        ]);
        let mut future_revisions = old_revisions;
        future_revisions.insert("demo_x".to_string(), 2);
        future_revisions.insert("demo_sin".to_string(), 2);

        let retained = retain_valid_series_extent_jobs(
            &[stable.clone(), demo.clone()],
            &future_revisions,
            &existing,
        )
        .unwrap();
        assert_eq!(retained.len(), 1);
        assert!(Rc::ptr_eq(
            retained.get(&stable_request.key).unwrap(),
            &stable_job
        ));

        let requests = active_series_extent_requests(&[stable, demo], &future_revisions).unwrap();
        let (_lazy_map, pending) =
            select_series_extent_job_map(requests, &retained, false).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].series.x_column, "demo_x");
        assert_eq!(pending[0].series.y_column, "demo_sin");
    }

    #[test]
    fn demo_precommit_abort_preserves_every_host_owned_authority() {
        let config = renderer::default::default_config();
        let series = vec![line_series("stable", "stable-x", "stable-y")];
        let columns = HashMap::from([("stable-x".to_string(), 2), ("stable-y".to_string(), 2)]);
        let revisions = HashMap::from([("stable-x".to_string(), 1), ("stable-y".to_string(), 1)]);
        let labels = vec![Some("stable".to_string())];
        let styles = vec![7u32];
        let color_seq = 1usize;
        let request = series_extent_request_from(&revisions, &series[0]).unwrap();
        let job = SeriesExtentJob::pending();
        let extents = HashMap::from([(request.key, Rc::clone(&job))]);

        let prepared_metadata =
            prepare_demo_column_metadata(&columns, &revisions, 2, [512; 4]).unwrap();
        let prepared_declarations = prepare_demo_declarations(
            &config,
            &series,
            &labels,
            renderer::ColorCycle::Classic,
            color_seq,
        )
        .unwrap();
        let prepared_extents = retain_valid_series_extent_jobs(
            &prepared_declarations.series,
            &prepared_metadata.revisions,
            &extents,
        )
        .unwrap();
        drop((prepared_metadata, prepared_declarations, prepared_extents));

        assert_eq!(config, renderer::default::default_config());
        assert_eq!(series, vec![line_series("stable", "stable-x", "stable-y")]);
        assert_eq!(columns.len(), 2);
        assert_eq!(revisions.len(), 2);
        assert_eq!(styles, vec![7]);
        assert_eq!(labels, vec![Some("stable".to_string())]);
        assert_eq!(color_seq, 1);
        assert_eq!(extents.len(), 1);
        assert!(Rc::ptr_eq(extents.values().next().unwrap(), &job));
    }

    #[test]
    fn column_registry_actions_fail_closed() {
        assert_eq!(
            validate_column_registry_action(false, ColumnRegistryAction::Register),
            Ok(())
        );
        assert_eq!(
            validate_column_registry_action(true, ColumnRegistryAction::Register),
            Err("is already registered")
        );
        assert_eq!(
            validate_column_registry_action(true, ColumnRegistryAction::Update),
            Ok(())
        );
        assert_eq!(
            validate_column_registry_action(false, ColumnRegistryAction::Update),
            Err("is not registered")
        );
    }

    #[test]
    fn column_data_length_is_validated() {
        assert_eq!(validate_column_data_len(0), Err("data must not be empty"));
        assert_eq!(validate_column_data_len(1), Ok(()));
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::{cell::Cell, collections::HashMap, rc::Rc};

    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::spawn_local;
    use web_sys::HtmlCanvasElement;

    use renderer::data_config::ErrorRef;
    use renderer::layout::{ChartArea, NudgeResult, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::{RichText, rich_segments_from_text};
    use renderer::{
        Chart, ChartDrawItem, ChartId, ChartRenderStamp, ChartStyle, ChartView, Color,
        ColumnSource, CpuTextMeasure, DataLineStyleConfig, DataRenderType, DefragPolicy, FitExtent,
        HiLoColumnSource, HitId, HitMap, Renderer, ResizeHandle as ModelResizeHandle, SelectionBox,
        RegisteredChartDrawItem, Series, SeriesConfig, StreamColumn, StreamEncoding,
        StreamRangeSourceBinding, StreamReplay, StreamSourceBinding, StreamStatistics,
        StreamingChartOptions, StreamingLimits, WindowedRenderer,
    };

    use renderer::{InitEvent, InitPhase};
    use renderer::data_render::{AllocError, GrowthPolicy};
    use renderer::gpu_memory::GpuResourceKind;

    use crate::borrowed_column::{
        BorrowedCastF32Column, BorrowedF32Column, BorrowedF64Column, BorrowedSplitF64Column,
        JsStreamColumn,
    };
    use crate::scalar_job::{SeriesExtentJob, SeriesFitExtent};
    use crate::{
        ColumnRegistryAction, FrameDecision, PreparedDemoDeclarations, SeriesExtentKey,
        active_series_extent_requests, consume_successful_frame, frame_decision,
        prepare_batch_column_metadata, prepare_column_metadata, prepare_demo_column_metadata,
        prepare_demo_declarations, retain_valid_series_extent_jobs, select_series_extent_job_map,
        series_extent_key_matches_config, series_extent_mode, validate_column_batch,
        validate_column_data_len, validate_column_registry_action,
    };

    const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

    fn js_err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
    }

    fn column_registration_error(error: renderer::FiggyError) -> JsValue {
        let (code, requested, limit, largest_free, total_free) = match &error {
            renderer::FiggyError::Pool(AllocError::OutOfSpace {
                requested,
                largest_free,
                total_free,
            }) => (
                "pool_space",
                Some(*requested),
                None,
                Some(*largest_free),
                Some(*total_free),
            ),
            renderer::FiggyError::Pool(AllocError::BudgetExceeded { requested, limit, .. }) => {
                ("budget_exceeded", Some(*requested), Some(*limit), None, None)
            }
            renderer::FiggyError::Pool(AllocError::ResourceLimit { requested, limit, .. })
            | renderer::FiggyError::GpuResourceLimit { requested, limit, .. } => {
                ("device_limit", Some(*requested), Some(*limit), None, None)
            }
            renderer::FiggyError::Pool(AllocError::AllocationFailed { .. })
            | renderer::FiggyError::GpuResourceAllocationFailed { .. } => {
                ("allocation_failed", None, None, None, None)
            }
            _ => ("registration_failed", None, None, None, None),
        };
        let js_error = js_sys::Error::new(&error.to_string());
        let object: &JsValue = js_error.as_ref();
        let set = |key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(object, &JsValue::from_str(key), &value);
        };
        set("code", JsValue::from_str(code));
        for (key, value) in [
            ("requestedBytes", requested),
            ("limitBytes", limit),
            ("largestFreeBytes", largest_free),
            ("totalFreeBytes", total_free),
        ] {
            if let Some(value) = value {
                let js_value = if value <= 9_007_199_254_740_991 {
                    JsValue::from_f64(value as f64)
                } else {
                    JsValue::from_str(&value.to_string())
                };
                set(key, js_value);
            }
        }
        js_error.into()
    }

    fn js_safe_u64(value: f64, name: &str) -> Result<u64, JsValue> {
        if !value.is_finite()
            || value < 0.0
            || value.fract() != 0.0
            || value > 9_007_199_254_740_991.0
        {
            return Err(js_err(format!(
                "{name} must be a non-negative safe integer"
            )));
        }
        Ok(value as u64)
    }

    fn js_stream_sources(values: &js_sys::Array) -> Result<Vec<JsStreamColumn>, JsValue> {
        let mut sources = Vec::new();
        sources
            .try_reserve_exact(values.length() as usize)
            .map_err(js_err)?;
        for index in 0..values.length() {
            sources.push(
                JsStreamColumn::from_js(&values.get(index))
                    .map_err(|reason| js_err(format!("stream source {index}: {reason}")))?,
            );
        }
        Ok(sources)
    }

    fn validate_stream_bindings(
        ids: &[String],
        revisions: &[f64],
        sources: &[JsStreamColumn],
    ) -> Result<Vec<u64>, JsValue> {
        if ids.len() != revisions.len() || ids.len() != sources.len() {
            return Err(js_err(
                "stream ids, revisions, and typed-array sources must have equal lengths",
            ));
        }
        let mut parsed = Vec::new();
        parsed.try_reserve_exact(revisions.len()).map_err(js_err)?;
        for (index, revision) in revisions.iter().copied().enumerate() {
            parsed.push(js_safe_u64(revision, &format!("stream revision {index}"))?);
        }
        Ok(parsed)
    }

    fn parse_stream_encoding(value: &str) -> Result<StreamEncoding, JsValue> {
        match value {
            "f32" => Ok(StreamEncoding::ScalarF32),
            "f64" => Ok(StreamEncoding::HiLoF32),
            _ => Err(js_err("stream encoding must be 'f32' or 'f64'")),
        }
    }

    fn stream_range_bindings<'a>(
        ids: &'a [String], revisions: &[f64], source_lengths: &[f64], offsets: &[f64],
        sources: &'a [JsStreamColumn],
    ) -> Result<Vec<StreamRangeSourceBinding<'a>>, JsValue> {
        if [revisions.len(), source_lengths.len(), offsets.len(), sources.len()]
            .into_iter().any(|len| len != ids.len())
        {
            return Err(js_err("range ids, revisions, source lengths, offsets, and typed arrays must have equal lengths"));
        }
        let mut bindings = Vec::new();
        bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
        for index in 0..ids.len() {
            bindings.push(StreamRangeSourceBinding {
                id: &ids[index],
                revision: js_safe_u64(revisions[index], "stream revision")?,
                source_len: js_safe_u64(source_lengths[index], "stream source length")?,
                offset: js_safe_u64(offsets[index], "stream offset")?,
                source: sources[index].source(),
            });
        }
        Ok(bindings)
    }

    fn stream_encoding_name(value: StreamEncoding) -> &'static str {
        match value {
            StreamEncoding::ScalarF32 => "f32",
            StreamEncoding::HiLoF32 => "f64",
        }
    }

    fn stream_columns_from_metadata(
        ids: &[String],
        revisions: &[f64],
        lengths: &[f64],
        encodings: &[String],
    ) -> Result<Vec<StreamColumn>, JsValue> {
        if ids.len() != revisions.len()
            || ids.len() != lengths.len()
            || ids.len() != encodings.len()
        {
            return Err(js_err(
                "stream ids, revisions, lengths, and encodings must have equal lengths",
            ));
        }
        let mut columns = Vec::new();
        columns.try_reserve_exact(ids.len()).map_err(js_err)?;
        for index in 0..ids.len() {
            columns.push(StreamColumn {
                id: ids[index].clone(),
                len: js_safe_u64(lengths[index], &format!("stream length {index}"))?,
                encoding: parse_stream_encoding(&encodings[index])?,
                replay: StreamReplay::RandomAccess,
                revision: js_safe_u64(revisions[index], &format!("stream revision {index}"))?,
                statistics: StreamStatistics::Unknown,
            });
        }
        Ok(columns)
    }

    #[cfg(test)]
    thread_local! {
        static LOAD_DEMO_ABORT_BEFORE_RENDERER_COMMIT: Cell<bool> = const { Cell::new(false) };
        static SERIES_EXTENT_SUBMIT_COUNT: Cell<u64> = const { Cell::new(0) };
    }

    #[cfg(test)]
    fn set_load_demo_abort_before_renderer_commit(enabled: bool) {
        LOAD_DEMO_ABORT_BEFORE_RENDERER_COMMIT.with(|state| state.set(enabled));
    }

    #[cfg(test)]
    fn record_series_extent_submit() {
        SERIES_EXTENT_SUBMIT_COUNT.with(|count| count.set(count.get().wrapping_add(1)));
    }

    #[cfg(test)]
    fn series_extent_submit_count() -> u64 {
        SERIES_EXTENT_SUBMIT_COUNT.with(Cell::get)
    }

    #[cfg(test)]
    fn reset_series_extent_submit_count() {
        SERIES_EXTENT_SUBMIT_COUNT.with(|count| count.set(0));
    }

    fn emit_init_progress(on_event: Option<&js_sys::Function>, event: InitEvent) {
        let Some(callback) = on_event else {
            return;
        };
        let payload = js_sys::Object::new();
        let phase = match event.phase {
            InitPhase::Started => "started",
            InitPhase::Finished => "finished",
        };
        let _ = js_sys::Reflect::set(&payload, &"scope".into(), &JsValue::from_str(event.scope));
        let _ = js_sys::Reflect::set(&payload, &"stage".into(), &JsValue::from_str(event.stage));
        let _ = js_sys::Reflect::set(&payload, &"phase".into(), &JsValue::from_str(phase));
        let _ = callback.call1(&JsValue::UNDEFINED, &payload);
    }

    async fn create_chart_kernel(
        canvas: HtmlCanvasElement,
        on_event: Option<js_sys::Function>,
    ) -> Result<FiggyChart, JsValue> {
        console_error_panic_hook::set_once();

        let (mut renderer, w, h) = {
            let on_event_ref = on_event.as_ref();
            let mut observer = |event| emit_init_progress(on_event_ref, event);
            let (w, h) = (canvas.width().max(1), canvas.height().max(1));
            let mut renderer = Renderer::for_window_async_observed(
                wgpu::SurfaceTarget::Canvas(canvas),
                (w, h),
                POOL_CAPACITY,
                &mut observer,
            )
            .await
            .map_err(js_err)?;
            renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
            (renderer, w, h)
        };

        emit_init_progress(
            on_event.as_ref(),
            InitEvent::new("web.create", "chart resources", InitPhase::Started),
        );

        let mut config = renderer::default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: w,
            height: h,
        });
        let chart_id = renderer
            .register_chart(config.clone(), Vec::new())
            .map_err(js_err)?;
        let chart = Chart::new(config);
        let view = renderer
            .create_chart_view(
                &chart,
                Rect {
                    x: 0,
                    y: 0,
                    width: w,
                    height: h,
                },
            )
            .map_err(js_err)?;

        emit_init_progress(
            on_event.as_ref(),
            InitEvent::new("web.create", "chart resources", InitPhase::Finished),
        );
        let mut chart = FiggyChart {
            renderer,
            chart_id,
            view,
            surface_size: (w, h),
            styles: Vec::new(),
            labels: Vec::new(),
            legend_auto_managed: true,
            columns: HashMap::new(),
            column_revisions: HashMap::new(),
            next_column_revision: 1,
            pending_stream_handoff: None,
            series_extents: HashMap::new(),
            alive: Rc::new(Cell::new(true)),
            color_seq: 0,
            hitmap: HitMap::standard_chart(),
            dragging: false,
            resizing: None,
            cycle: renderer::ColorCycle::Classic,
            clear_color: Color::WHITE,
            view_dirty: false,
            redraw_pending: true,
            last_presented_stamp: None,
        };

        emit_init_progress(
            on_event.as_ref(),
            InitEvent::new("web.create", "first frame", InitPhase::Started),
        );
        chart.first_frame_ready().await?;
        emit_init_progress(
            on_event.as_ref(),
            InitEvent::new("web.create", "first frame", InitPhase::Finished),
        );
        Ok(chart)
    }

    /// Parameter metadata for one `draw_style` mode — a JSON array of
    /// `{key, min, max, default, integer}`. Ranges are the RECOMMENDED
    /// slider spans (the SSoT accepts values beyond them; the renderer
    /// applies only safety guards), and they come from the model crate, so
    /// hosts never hardcode them.
    ///
    /// JS: `const specs = JSON.parse(draw_style_param_specs("constellation"));`
    /// Valid modes: `draw_style_modes()`.
    #[wasm_bindgen]
    pub fn draw_style_param_specs(mode: &str) -> Result<String, JsValue> {
        let specs = renderer::config::DrawStyle::param_specs_for_mode(mode).ok_or_else(|| {
            js_err(format!(
                "unknown draw_style mode {mode:?} (valid: {})",
                renderer::config::DrawStyle::mode_tags().join(", ")
            ))
        })?;
        serde_json::to_string(specs).map_err(js_err)
    }

    /// Every valid `draw_style` mode tag, as a JSON string array.
    #[wasm_bindgen]
    pub fn draw_style_modes() -> Result<String, JsValue> {
        serde_json::to_string(renderer::config::DrawStyle::mode_tags()).map_err(js_err)
    }

    /// Every column id a series' render type references (x/y + error refs).
    fn referenced_columns(cfg: &SeriesConfig) -> Vec<&str> {
        fn push_ref<'a>(ids: &mut Vec<&'a str>, r: &'a ErrorRef) {
            match r {
                ErrorRef::Symmetric { column } => ids.push(column),
                ErrorRef::Asymmetric { lower, upper } => {
                    ids.push(lower);
                    ids.push(upper);
                }
            }
        }

        fn push_scatter_ref<'a>(
            ids: &mut Vec<&'a str>,
            scatter: &'a renderer::DataScatterStyleConfig,
        ) {
            if let Some(column) = &scatter.point_style_index_column {
                ids.push(column);
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
        }
        ids
    }

    enum ColumnUploadSource<'a> {
        Scalar(&'a dyn ColumnSource),
        HiLo(&'a dyn HiLoColumnSource),
    }

    struct PreparedSeriesExtentCache {
        extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
        ticket_jobs: Vec<(renderer::GpuSeriesExtentTicket, Rc<SeriesExtentJob>)>,
    }

    // ------------------------------------------------------------------
    // Preset mirrors — fieldless enums cross the boundary as integers.
    // ------------------------------------------------------------------

    /// Axis frame presets (mirror of `model::AxisPreset`).
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum AxisPreset {
        BoxedInward,
        BoxedOutward,
        OpenOutward,
        OpenInward,
        Minimal,
    }

    impl From<AxisPreset> for renderer::AxisPreset {
        fn from(p: AxisPreset) -> Self {
            match p {
                AxisPreset::BoxedInward => Self::BoxedInward,
                AxisPreset::BoxedOutward => Self::BoxedOutward,
                AxisPreset::OpenOutward => Self::OpenOutward,
                AxisPreset::OpenInward => Self::OpenInward,
                AxisPreset::Minimal => Self::Minimal,
            }
        }
    }

    /// Series color rotations (mirror of `model::ColorCycle`).
    #[wasm_bindgen]
    #[derive(Clone, Copy)]
    pub enum ColorCycle {
        Classic,
        Vivid,
        Balanced,
        ColorblindSafe,
        Monochrome,
    }

    impl From<ColorCycle> for renderer::ColorCycle {
        fn from(c: ColorCycle) -> Self {
            match c {
                ColorCycle::Classic => Self::Classic,
                ColorCycle::Vivid => Self::Vivid,
                ColorCycle::Balanced => Self::Balanced,
                ColorCycle::ColorblindSafe => Self::ColorblindSafe,
                ColorCycle::Monochrome => Self::Monochrome,
            }
        }
    }

    /// Result of one non-blocking exact streaming step.
    #[wasm_bindgen]
    pub struct StreamingStepResult {
        status: &'static str,
        revision: String,
        submitted_primitives: f64,
        total_primitives: f64,
        pending_latest: bool,
    }

    #[wasm_bindgen]
    impl StreamingStepResult {
        #[wasm_bindgen(getter)]
        pub fn revision(&self) -> String { self.revision.clone() }
        #[wasm_bindgen(getter)]
        pub fn status(&self) -> String {
            self.status.to_owned()
        }

        #[wasm_bindgen(getter)]
        pub fn submitted_primitives(&self) -> f64 {
            self.submitted_primitives
        }

        #[wasm_bindgen(getter)]
        pub fn total_primitives(&self) -> f64 {
            self.total_primitives
        }

        #[wasm_bindgen(getter)]
        pub fn pending_latest(&self) -> bool {
            self.pending_latest
        }
    }

    /// Result of asking the renderer to start or coalesce one automatic draw.
    #[wasm_bindgen]
    pub struct AutoStreamingRequestResult {
        status: &'static str,
        revision: String,
        job_id: Option<String>,
        submitted_primitives: f64,
        total_primitives: f64,
        source_ids: Vec<String>,
        source_revisions: Vec<f64>,
        pending_latest: bool,
    }

    #[wasm_bindgen]
    impl AutoStreamingRequestResult {
        #[wasm_bindgen(getter)]
        pub fn revision(&self) -> String { self.revision.clone() }
        #[wasm_bindgen(getter)]
        pub fn job_id(&self) -> Option<String> { self.job_id.clone() }
        #[wasm_bindgen(getter)]
        pub fn submitted_primitives(&self) -> f64 { self.submitted_primitives }
        #[wasm_bindgen(getter)]
        pub fn total_primitives(&self) -> f64 { self.total_primitives }
        #[wasm_bindgen(getter)]
        pub fn status(&self) -> String {
            self.status.to_owned()
        }

        #[wasm_bindgen(getter)]
        pub fn source_ids(&self) -> Vec<String> {
            self.source_ids.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn source_revisions(&self) -> Vec<f64> {
            self.source_revisions.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn pending_latest(&self) -> bool {
            self.pending_latest
        }
    }

    /// Exact range selected by the renderer-owned automatic cursor.
    #[wasm_bindgen]
    pub struct AutoStreamingRangeRequestResult {
        selection_ticket: Option<renderer::StreamingSelectionTicket>,
        status: &'static str,
        revision: String,
        source_ids: Vec<String>,
        source_revisions: Vec<f64>,
        source_lengths: Vec<f64>,
        offsets: Vec<f64>,
        lengths: Vec<f64>,
        encodings: Vec<String>,
        submitted_primitives: f64,
        total_primitives: f64,
        pending_latest: bool,
    }

    impl AutoStreamingRangeRequestResult {
        fn from_core(
            request: renderer::AutoStreamingRangeRequest,
            revision: String,
            complete_counts: (f64, f64),
        ) -> Result<Self, JsValue> {
            let mut response = Self {
                selection_ticket: None,
                status: "backpressure", revision,
                source_ids: Vec::new(), source_revisions: Vec::new(),
                source_lengths: Vec::new(), offsets: Vec::new(), lengths: Vec::new(),
                encodings: Vec::new(), submitted_primitives: 0.0,
                total_primitives: 0.0, pending_latest: false,
            };
            match request {
                renderer::AutoStreamingRangeRequest::Ready {
                    submitted_primitives, total_primitives, ranges, ..
                } => {
                    response.status = "ready";
                    response.submitted_primitives = submitted_primitives as f64;
                    response.total_primitives = total_primitives as f64;
                    response.source_ids.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    response.source_revisions.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    response.source_lengths.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    response.offsets.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    response.lengths.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    response.encodings.try_reserve_exact(ranges.len()).map_err(js_err)?;
                    for range in ranges {
                        response.source_ids.push(range.id);
                        response.source_revisions.push(range.revision as f64);
                        response.source_lengths.push(range.source_len as f64);
                        response.offsets.push(range.offset as f64);
                        response.lengths.push(range.len as f64);
                        response.encodings.push(stream_encoding_name(range.encoding).to_owned());
                    }
                }
                renderer::AutoStreamingRangeRequest::Backpressure {
                    submitted_primitives, total_primitives, ..
                } => {
                    response.submitted_primitives = submitted_primitives as f64;
                    response.total_primitives = total_primitives as f64;
                }
                renderer::AutoStreamingRangeRequest::AllSubmitted { total_primitives, .. } => {
                    response.status = "all_submitted";
                    response.submitted_primitives = total_primitives as f64;
                    response.total_primitives = total_primitives as f64;
                }
                renderer::AutoStreamingRangeRequest::Complete { pending_latest, .. } => {
                    response.status = "complete";
                    response.submitted_primitives = complete_counts.0;
                    response.total_primitives = complete_counts.1;
                    response.pending_latest = pending_latest.is_some();
                }
            }
            Ok(response)
        }
    }

    /// Opaque renderer-issued replay operation. Contains no column payload.
    #[wasm_bindgen]
    pub struct StreamingOperationHandle {
        operation: renderer::StreamingOperation,
        revision: String,
        counts: Cell<(f64, f64)>,
    }

    #[wasm_bindgen]
    impl AutoStreamingRangeRequestResult {
        pub fn same_selection_request(&self, other: &AutoStreamingRangeRequestResult) -> bool {
            self.selection_ticket.is_some() && self.selection_ticket == other.selection_ticket
        }

        #[wasm_bindgen(getter)]
        pub fn revision(&self) -> String { self.revision.clone() }
        #[wasm_bindgen(getter)]
        pub fn status(&self) -> String {
            self.status.to_owned()
        }

        #[wasm_bindgen(getter)]
        pub fn source_ids(&self) -> Vec<String> {
            self.source_ids.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn source_revisions(&self) -> Vec<f64> {
            self.source_revisions.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn source_lengths(&self) -> Vec<f64> {
            self.source_lengths.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn offsets(&self) -> Vec<f64> {
            self.offsets.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn lengths(&self) -> Vec<f64> {
            self.lengths.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn encodings(&self) -> Vec<String> {
            self.encodings.clone()
        }

        #[wasm_bindgen(getter)]
        pub fn submitted_primitives(&self) -> f64 {
            self.submitted_primitives
        }

        #[wasm_bindgen(getter)]
        pub fn total_primitives(&self) -> f64 {
            self.total_primitives
        }

        #[wasm_bindgen(getter)]
        pub fn pending_latest(&self) -> bool {
            self.pending_latest
        }
    }

    fn resident_admission_reason(status: renderer::ResidentAdmissionStatus) -> &'static str {
        match status {
            renderer::ResidentAdmissionStatus::Admissible => "admissible",
            renderer::ResidentAdmissionStatus::MemoryBudgetUnset => "memory_budget_unset",
            renderer::ResidentAdmissionStatus::ColumnLengthLimitExceeded { .. } => {
                "column_length_limit"
            }
            renderer::ResidentAdmissionStatus::DerivedBufferLimitExceeded { .. } => {
                "derived_buffer_limit"
            }
            renderer::ResidentAdmissionStatus::ArithmeticOverflow => "arithmetic_overflow",
            renderer::ResidentAdmissionStatus::WorkingSetLimitExceeded => {
                "working_set_limit"
            }
            renderer::ResidentAdmissionStatus::DeviceBufferLimitExceeded => {
                "device_buffer_limit"
            }
            renderer::ResidentAdmissionStatus::PoolCapacityUnavailable => {
                "pool_capacity_unavailable"
            }
            renderer::ResidentAdmissionStatus::MemoryBudgetExceeded => "memory_budget",
        }
    }

    /// Current bounded renderer-owned streaming reservations.
    #[wasm_bindgen]
    pub struct StreamingUsageResult {
        active_charts: u32,
        in_flight_chunks: u32,
        reserved_gpu_bytes: f64,
    }

    #[wasm_bindgen]
    impl StreamingUsageResult {
        #[wasm_bindgen(getter)]
        pub fn active_charts(&self) -> u32 {
            self.active_charts
        }

        #[wasm_bindgen(getter)]
        pub fn in_flight_chunks(&self) -> u32 {
            self.in_flight_chunks
        }

        #[wasm_bindgen(getter)]
        pub fn reserved_gpu_bytes(&self) -> f64 {
            self.reserved_gpu_bytes
        }
    }

    /// CSS color strings (`"rgb(r g b / a)"`) for a cycle — lets the host UI
    /// render swatches / legends with exactly the chart's palette.
    #[wasm_bindgen]
    pub fn color_cycle_css(cycle: ColorCycle) -> Vec<String> {
        renderer::ColorCycle::from(cycle)
            .colors()
            .iter()
            .map(|c| {
                format!(
                    "rgb({} {} {} / {})",
                    (c.r * 255.0).round() as u8,
                    (c.g * 255.0).round() as u8,
                    (c.b * 255.0).round() as u8,
                    c.a,
                )
            })
            .collect()
    }

    // ------------------------------------------------------------------
    // FiggyChart — low-level wasm kernel bound to one canvas.
    // ------------------------------------------------------------------

    struct PendingStreamHandoffMetadata {
        columns: Vec<(String, usize, u64)>,
        next_column_revision: u64,
    }

    #[wasm_bindgen]
    pub struct FiggyChart {
        renderer: WindowedRenderer<'static>,
        chart_id: ChartId,
        view: ChartView,
        /// Current WebGPU surface size in physical canvas pixels. This is a
        /// viewport property, not the exported document size.
        surface_size: (u32, u32),
        styles: Vec<ChartStyle>,
        /// 1:1 with renderer-owned series order. Legacy plain-label fallback
        /// for explicit legend reset; rendered series/config remain the SSoT.
        labels: Vec<Option<String>>,
        /// True while the legend follows the wrapper's auto row layout.
        /// Direct `set_config` legend edits turn this off so later series
        /// changes preserve user-authored text instead of deleting rows.
        legend_auto_managed: bool,
        /// Registered columns: id to logical value count.
        columns: HashMap<String, usize>,
        /// Successful changed-upload revision for each live GPU column.
        column_revisions: HashMap<String, u64>,
        next_column_revision: u64,
        /// Prepared wrapper metadata for a core handoff that is not live
        /// until the renderer reports its atomic completion.
        pending_stream_handoff: Option<PendingStreamHandoffMetadata>,
        /// Already-submitted exact GPU drawable-domain reductions. Cache
        /// identity includes the normalized primitive mode and every active
        /// role-specific column revision.
        series_extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
        alive: Rc<Cell<bool>>,
        /// Monotonic color assignment for newly registered series.
        color_seq: usize,
        hitmap: HitMap,
        dragging: bool,
        resizing: Option<ModelResizeHandle>,
        cycle: renderer::ColorCycle,
        clear_color: Color,
        view_dirty: bool,
        /// Host-owned surface state not represented by renderer chart
        /// revisions (currently the clear color). Starts true so the initial
        /// surface frame is never skipped.
        redraw_pending: bool,
        /// Renderer-issued identity of the state last presented onscreen.
        last_presented_stamp: Option<ChartRenderStamp>,
    }

    impl FiggyChart {
        fn chart_config(&self) -> &renderer::Config {
            self.renderer
                .chart_config(self.chart_id)
                .expect("FiggyChart keeps its renderer-owned chart registered")
        }

        fn chart_series(&self) -> &[SeriesConfig] {
            self.renderer
                .chart_series(self.chart_id)
                .expect("FiggyChart keeps its renderer-owned chart registered")
        }

        fn chart_selection(&self) -> Option<HitId> {
            self.renderer
                .chart_selection(self.chart_id)
                .expect("FiggyChart keeps its renderer-owned chart registered")
        }

        fn replace_chart_config(&mut self, config: renderer::Config) -> Result<(), JsValue> {
            self.renderer
                .set_chart_config(self.chart_id, config)
                .map_err(js_err)
        }

        fn replace_chart_state(
            &mut self,
            config: renderer::Config,
            series: Vec<SeriesConfig>,
        ) -> Result<(), JsValue> {
            self.renderer
                .set_chart_state(self.chart_id, config, series)
                .map_err(js_err)
        }

        fn display_scale_and_panel(&self) -> (f32, Rect) {
            let logical = self.chart_config().chart_area.0;
            renderer::fit_display_panel((logical.width, logical.height), self.surface_size)
        }

        fn display_config(&self) -> (renderer::config::Config, Rect, f32) {
            renderer::display_config_for_surface(self.chart_config(), self.surface_size)
        }

        fn display_delta_to_document(&self, dx: f32, dy: f32) -> (f32, f32) {
            let (scale, _) = self.display_scale_and_panel();
            if scale > 0.0 {
                (dx / scale, dy / scale)
            } else {
                (dx, dy)
            }
        }

        fn label_text(&self, label: &str) -> RichText {
            let content = &self.chart_config().legend.content;
            RichText {
                segments: rich_segments_from_text(label),
                color: content.color,
                font_size: content.font_size,
                font: content.font.clone(),
            }
        }

        fn rich_text_to_plain(rt: &RichText) -> String {
            rt.segments.iter().map(|s| s.text).collect()
        }

        fn fallback_label(&self, i: usize) -> Option<RichText> {
            self.chart_series()
                .get(i)
                .and_then(|cfg| cfg.label.clone())
                .or_else(|| self.labels.get(i)?.as_ref().map(|s| self.label_text(s)))
        }

        /// Explicit reset: rebuild every auto legend row from SeriesConfig.label
        /// first, falling back to the wrapper's legacy string labels.
        fn rebuild_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
            let entries: Vec<(SeriesConfig, RichText)> = self
                .chart_series()
                .iter()
                .cloned()
                .enumerate()
                .filter_map(|(i, cfg)| Some((cfg, self.fallback_label(i)?)))
                .collect();

            let mut config = self.chart_config().clone();
            config.legend.content.segments.clear();
            for (cfg, label) in entries {
                renderer::config::append_legend_entry_rich(
                    &mut config.legend.content,
                    renderer::config::series_symbol_segments(&cfg),
                    label.segments,
                );
            }
            config.legend.visible = !config.legend.content.segments.is_empty();
            self.replace_chart_config(config)?;
            self.legend_auto_managed = true;
            Ok(())
        }

        fn rebuild_styles(&mut self) {
            let (scale, _) = self.display_scale_and_panel();
            self.styles = self
                .chart_series()
                .iter()
                .map(|cfg| self.renderer.create_style_for_series_scaled(cfg, scale))
                .collect();
        }

        fn request_host_redraw(&mut self) {
            self.redraw_pending = true;
        }
        fn publish_series_extents_if_engine_ready(
            &mut self,
            series_cfgs: &[SeriesConfig],
            revisions: &HashMap<String, u64>,
            retry_failed: bool,
        ) -> Result<(), JsValue> {
            if !self.renderer.errorbar_extent_engine_ready() {
                return Ok(());
            }
            let prepared =
                self.prepare_series_extent_cache(series_cfgs, revisions, retry_failed)?;
            self.publish_series_extent_cache(prepared);
            Ok(())
        }

        fn prepare_series_extent_cache(
            &self,
            series_cfgs: &[SeriesConfig],
            revisions: &HashMap<String, u64>,
            retry_failed: bool,
        ) -> Result<PreparedSeriesExtentCache, JsValue> {
            let mut resident_series = Vec::new();
            resident_series
                .try_reserve(series_cfgs.len())
                .map_err(js_err)?;
            resident_series.extend(
                series_cfgs
                    .iter()
                    .filter(|series| {
                        referenced_columns(series).into_iter().all(|id| {
                            matches!(
                                self.renderer.logical_column(id),
                                Some(renderer::LogicalColumn::Resident(_))
                            )
                        })
                    })
                    .cloned(),
            );
            let requests =
                active_series_extent_requests(&resident_series, revisions).map_err(js_err)?;
            let (extents, pending) =
                select_series_extent_job_map(requests, &self.series_extents, retry_failed)
                    .map_err(js_err)?;
            let mut ticket_jobs = Vec::new();
            ticket_jobs.try_reserve(pending.len()).map_err(js_err)?;

            for request in pending {
                let job = extents.get(&request.key).cloned().ok_or_else(|| {
                    js_err("pending series extent key lost its prepared job before publication")
                })?;
                match self.renderer.begin_series_fit_extent(&request.series) {
                    Ok(Some(ticket)) => {
                        #[cfg(test)]
                        record_series_extent_submit();
                        ticket_jobs.push((ticket, job));
                    }
                    Ok(None) => job.complete_terminal_failure(
                        "active GPU extent request resolved to a metadata-only series".into(),
                    ),
                    Err(error) => job.complete_terminal_failure(error.to_string()),
                }
            }

            Ok(PreparedSeriesExtentCache {
                extents,
                ticket_jobs,
            })
        }

        fn publish_series_extent_cache(&mut self, prepared: PreparedSeriesExtentCache) {
            self.series_extents = prepared.extents;
            for (ticket, job) in prepared.ticket_jobs {
                Self::spawn_series_extent(ticket, job);
            }
        }

        /// Removing series cannot create a new extent identity. Retire stale
        /// jobs without allocating or submitting new GPU work.
        fn retire_series_extent_cache(&mut self) {
            let series_cfgs = self
                .renderer
                .chart_series(self.chart_id)
                .expect("FiggyChart keeps its renderer-owned chart registered");
            let revisions = &self.column_revisions;
            self.series_extents.retain(|key, _| {
                series_cfgs
                    .iter()
                    .any(|cfg| series_extent_key_matches_config(key, cfg, revisions))
            });
        }

        fn spawn_series_extent(ticket: renderer::GpuSeriesExtentTicket, job: Rc<SeriesExtentJob>) {
            spawn_local(async move {
                match ticket.resolve().await {
                    Ok(extent) => job.complete_success(extent.map(|extent| SeriesFitExtent {
                        x: FitExtent {
                            min: extent.x.min,
                            max: extent.x.max,
                            min_positive: extent.x.min_positive,
                        },
                        y: FitExtent {
                            min: extent.y.min,
                            max: extent.y.max,
                            min_positive: extent.y.min_positive,
                        },
                    })),
                    Err(error) => job.complete_retryable_failure(error.to_string()),
                }
            });
        }

        fn upsert_column_atomic(
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
                ColumnUploadSource::HiLo(source) => {
                    self.renderer.begin_upsert_hilo_column(id, source)
                }
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
        fn commit_column_batch<S: ColumnSource>(
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
            self.renderer.add_columns(&refs).map_err(column_registration_error)?;
            self.columns = metadata.columns;
            self.column_revisions = metadata.revisions;
            self.next_column_revision = metadata.next_revision;
            Ok(())
        }

        fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
            for id in referenced_columns(cfg) {
                if self.renderer.logical_column(id).is_none() {
                    return Err(js_err(format!(
                        "series '{}' references unregistered column '{id}'",
                        cfg.series_id
                    )));
                }
            }
            Ok(())
        }
    }

    impl Drop for FiggyChart {
        fn drop(&mut self) {
            self.alive.set(false);
        }
    }

    #[wasm_bindgen]
    impl FiggyChart {
        /// Bind the low-level chart kernel to `canvas` (uses the canvas's
        /// current pixel size). Ordinary web hosts should prefer
        /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
        /// JS: `const chart = await FiggyChart.create(canvas);`
        pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
            create_chart_kernel(canvas, None).await
        }

        /// Same as [`Self::create`], reporting each init stage to `on_event`
        /// as `{ scope, stage, phase }`. Do not call other kernel methods
        /// from the callback.
        pub async fn create_with_progress(
            canvas: HtmlCanvasElement,
            on_event: js_sys::Function,
        ) -> Result<FiggyChart, JsValue> {
            create_chart_kernel(canvas, Some(on_event)).await
        }

        /// Register a font (TTF/OTF/TTC bytes) for SSoT `font` family names.
        /// Returns the family names the file declares — use them verbatim in
        /// `content.font` / label styles. Registered fonts win over native
        /// system fonts, so resolution behaves identically on web and
        /// desktop. Already-drawn text re-rasterizes on the next `frame()`.
        ///
        /// JS: `chart.register_font(new Uint8Array(await (await fetch(url)).arrayBuffer()))`
        pub fn register_font(&mut self, bytes: &[u8]) -> Result<Vec<String>, JsValue> {
            let families =
                renderer::text_render::register_font_bytes(bytes.to_vec()).map_err(js_err)?;
            // Text may already be on screen in the fallback font — force a
            // decoration re-raster so the registration is visible.
            Ok(families)
        }

        // ---- exact non-resident streaming ----

        /// Install the renderer's bounded streaming resources once.
        /// Byte limits are JavaScript safe integers; no source payload is retained.
        pub fn configure_streaming(
            &mut self,
            max_active_charts: u32,
            max_in_flight_chunks: u32,
            max_columns_per_chunk: u32,
            max_chunk_input_bytes: f64,
            max_in_flight_gpu_bytes: f64,
        ) -> Result<(), JsValue> {
            self.renderer
                .configure_streaming(StreamingLimits {
                    max_active_charts: max_active_charts as usize,
                    max_in_flight_chunks: max_in_flight_chunks as usize,
                    max_columns_per_chunk: max_columns_per_chunk as usize,
                    max_chunk_input_bytes: js_safe_u64(
                        max_chunk_input_bytes,
                        "max_chunk_input_bytes",
                    )?,
                    max_in_flight_gpu_bytes: js_safe_u64(
                        max_in_flight_gpu_bytes,
                        "max_in_flight_gpu_bytes",
                    )?,
                })
                .map_err(js_err)
        }

        /// Configure the total GPU budget and chart-local packed-view limit.
        /// Automatic streaming keeps original sources nonresident, then caches
        /// only source rows whose geometry reaches the current view when the
        /// completed packed view fits both limits.
        pub fn configure_auto_residency(
            &mut self,
            memory_budget_bytes: f64,
            working_set_limit_bytes: f64,
        ) -> Result<(), JsValue> {
            let memory_budget_bytes =
                js_safe_u64(memory_budget_bytes, "memory_budget_bytes")?;
            let working_set_limit_bytes =
                js_safe_u64(working_set_limit_bytes, "working_set_limit_bytes")?;
            let current = self.renderer.gpu_memory_usage().total_bytes();
            if memory_budget_bytes < current {
                return Err(js_err(format!(
                    "memory_budget_bytes {memory_budget_bytes} is below current renderer usage {current}"
                )));
            }
            let exceeded = self.renderer.set_memory_budget(Some(memory_budget_bytes));
            debug_assert!(exceeded.is_none());
            self.renderer
                .set_auto_resident_working_set_limit(Some(working_set_limit_bytes));
            Ok(())
        }

        /// Opt in to pool growth on an otherwise out-of-space upload. Growth
        /// remains bounded by the device storage-binding limit and the
        /// configured renderer memory budget.
        pub fn set_pool_auto_growth(&mut self, enabled: bool) {
            self.renderer.set_pool_growth_policy(if enabled {
                GrowthPolicy::OnAllocFailure
            } else {
                GrowthPolicy::Fixed
            });
        }

        /// Read-only accounting. These are requested GPU allocations, not a
        /// driver VRAM measurement; retired bytes remain charged until the
        /// queue completion callback fires.
        pub fn gpu_memory_status(&self) -> String {
            let usage = self.renderer.gpu_memory_usage();
            let pool = self.renderer.pool();
            let mut sources: Vec<_> = self
                .columns
                .iter()
                .map(|(id, &length)| {
                    let (residency, encoded_bytes) = match self.renderer.logical_column(id) {
                        Some(renderer::LogicalColumn::Resident(slot)) => {
                            ("resident", Some(slot.byte_size))
                        }
                        Some(renderer::LogicalColumn::Streamed(_)) => ("streamed", None),
                        None => ("missing", None),
                    };
                    serde_json::json!({
                        "id": id,
                        "length": length,
                        "revision": self.column_revisions.get(id).map(u64::to_string),
                        "residency": residency,
                        "encoded_bytes": encoded_bytes,
                    })
                })
                .collect();
            sources.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
            let resources: Vec<_> = GpuResourceKind::ALL
                .into_iter()
                .map(|kind| serde_json::json!({
                    "kind": kind.label(),
                    "live_bytes": usage.live_bytes_of(kind),
                    "retired_bytes": usage.retired_bytes_of(kind),
                }))
                .collect();
            serde_json::json!({
                "total_bytes": usage.total_bytes(),
                "live_bytes": usage.live_bytes(),
                "retired_bytes": usage.retired_bytes(),
                "memory_budget_bytes": self.renderer.memory_budget(),
                "pool": {
                    "capacity_bytes": pool.capacity(),
                    "used_bytes": pool.used_bytes(),
                    "free_bytes": pool.free_bytes(),
                    "largest_free_bytes": pool.largest_free_region(),
                    "backup_bytes": pool.backup_bytes(),
                    "retired_bytes": pool.retired_bytes(),
                    "auto_growth": pool.growth_policy == GrowthPolicy::OnAllocFailure,
                },
                "resources": resources,
                "sources": sources,
            }).to_string()
        }

        /// Compact live resident columns, drop the unused pool slab, and wait
        /// for the device completion fence before reporting reclaimed bytes.
        /// The 16 MiB startup slab remains as the minimum reusable pool.
        pub async fn release_unused_gpu_memory(&mut self) -> Result<String, JsValue> {
            let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
            if matches!(
                status.status,
                renderer::StreamingState::Active
                    | renderer::StreamingState::AllSubmitted
                    | renderer::StreamingState::Cancelling
            ) {
                return Err(js_err("finish or cancel the active stream before GPU pool cleanup"));
            }
            self.renderer
                .release_unused_pool_memory(POOL_CAPACITY)
                .await
                .map_err(column_registration_error)?;
            Ok(self.gpu_memory_status())
        }

        /// Ask the renderer to draw the latest chart state. Decoration-only
        /// changes reuse the active accumulation; data, series, view, target,
        /// and chunk-budget changes replace it after successful preflight.
        pub fn request_auto_streaming_chart(
            &mut self,
            max_primitives_per_chunk: f64,
        ) -> Result<AutoStreamingRequestResult, JsValue> {
            let max_primitives_per_chunk = js_safe_u64(
                max_primitives_per_chunk,
                "max_primitives_per_chunk",
            )?;
            let (display_config, _, display_scale) = self.display_config();
            let result = self
                .renderer
                .request_auto_streaming_chart_with_display_scale(
                    self.chart_id,
                    &self.view,
                    display_config,
                    display_scale,
                    StreamingChartOptions {
                        size: self.surface_size,
                        clear_color: self.clear_color,
                        max_primitives_per_chunk,
                    },
                )
                .map_err(js_err)?;
            if matches!(&result, renderer::AutoStreamingRequest::Started { .. }) {
                self.request_host_redraw();
            }
            let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
            Ok(match result {
                renderer::AutoStreamingRequest::Started { sources, .. } => {
                    let mut source_ids = Vec::new();
                    let mut source_revisions = Vec::new();
                    source_ids.try_reserve_exact(sources.len()).map_err(js_err)?;
                    source_revisions
                        .try_reserve_exact(sources.len())
                        .map_err(js_err)?;
                    for source in sources {
                        source_ids.push(source.id);
                        source_revisions.push(source.revision as f64);
                    }
                    AutoStreamingRequestResult {
                        status: "started",
                        revision: status.revision.to_string(),
                        job_id: status.job_id.map(|id| id.to_string()),
                        submitted_primitives: status.submitted_primitives as f64,
                        total_primitives: status.total_primitives as f64,
                        source_ids,
                        source_revisions,
                        pending_latest: false,
                    }
                }
                renderer::AutoStreamingRequest::Active { pending_latest, .. } => {
                    AutoStreamingRequestResult {
                        status: "active",
                        revision: status.revision.to_string(),
                        job_id: status.job_id.map(|id| id.to_string()),
                        submitted_primitives: status.submitted_primitives as f64,
                        total_primitives: status.total_primitives as f64,
                        source_ids: Vec::new(),
                        source_revisions: Vec::new(),
                        pending_latest: pending_latest.is_some(),
                    }
                }
                renderer::AutoStreamingRequest::Complete { pending_latest, .. } => {
                    AutoStreamingRequestResult {
                        status: "complete",
                        revision: status.revision.to_string(),
                        job_id: status.job_id.map(|id| id.to_string()),
                        submitted_primitives: status.submitted_primitives as f64,
                        total_primitives: status.total_primitives as f64,
                        source_ids: Vec::new(),
                        source_revisions: Vec::new(),
                        pending_latest: pending_latest.is_some(),
                    }
                }
            })
        }

        /// Advance at most one renderer-selected automatic chunk. The typed
        /// arrays are borrowed only for this call.
        pub fn auto_stream_chart_step(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<StreamingStepResult, JsValue> {
            let sources = js_stream_sources(&sources)?;
            let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
            let mut bindings = Vec::new();
            bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
            for ((id, revision), source) in ids.iter().zip(revisions).zip(&sources) {
                bindings.push(StreamSourceBinding {
                    id,
                    revision,
                    source: source.source(),
                });
            }
            let progress = self
                .renderer
                .auto_stream_chart_step(self.chart_id, &bindings)
                .map_err(js_err)?;
            let result = match progress {
                renderer::AutoStreamingProgress::Submitted {
                    submitted_primitives,
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "submitted",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::Backpressure {
                    submitted_primitives,
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "backpressure",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::AllSubmitted {
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "all_submitted",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: total_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::Complete { pending_latest, .. } => {
                    let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
                    StreamingStepResult {
                        status: "complete",
                        revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                        submitted_primitives: counts.submitted_primitives as f64,
                        total_primitives: counts.total_primitives as f64,
                        pending_latest: pending_latest.is_some(),
                    }
                }
            };
            self.request_host_redraw();
            Ok(result)
        }

        /// Register replayable stream metadata without requiring a complete
        /// typed array at the wasm boundary. Payload ranges are supplied only
        /// after the renderer requests them.
        pub fn register_streaming_column_sources(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            lengths: Vec<f64>,
            encodings: Vec<String>,
        ) -> Result<(), JsValue> {
            let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
            let mut next_columns = self.columns.clone();
            let mut next_revisions = self.column_revisions.clone();
            next_columns.try_reserve(ids.len()).map_err(js_err)?;
            next_revisions.try_reserve(ids.len()).map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            for source in &columns {
                next_columns.insert(
                    source.id.clone(),
                    usize::try_from(source.len).map_err(js_err)?,
                );
                next_revisions.insert(source.id.clone(), source.revision);
                next_column_revision = next_column_revision.max(
                    source
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            self.renderer
                .register_streamed_columns(columns)
                .map_err(js_err)?;
            self.columns = next_columns;
            self.column_revisions = next_revisions;
            self.next_column_revision = next_column_revision;
            Ok(())
        }

        /// Replace existing stream metadata. The new revision becomes live
        /// only after the renderer accepts the whole batch.
        pub fn replace_streaming_column_sources(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            lengths: Vec<f64>,
            encodings: Vec<String>,
        ) -> Result<(), JsValue> {
            let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
            let mut next_columns = self.columns.clone();
            let mut next_revisions = self.column_revisions.clone();
            next_columns.try_reserve(ids.len()).map_err(js_err)?;
            next_revisions.try_reserve(ids.len()).map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            for source in &columns {
                next_columns.insert(
                    source.id.clone(),
                    usize::try_from(source.len).map_err(js_err)?,
                );
                next_revisions.insert(source.id.clone(), source.revision);
                next_column_revision = next_column_revision.max(
                    source
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            self.renderer
                .replace_streamed_columns(columns)
                .map_err(js_err)?;
            self.columns = next_columns;
            self.column_revisions = next_revisions;
            self.next_column_revision = next_column_revision;
            self.retire_series_extent_cache();
            Ok(())
        }

        /// Start a two-phase resident or mixed closure replacement. Existing
        /// authority remains live until the range cursor completes.
        pub fn request_resident_stream_handoff(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            lengths: Vec<f64>,
            encodings: Vec<String>,
            max_primitives_per_chunk: f64,
        ) -> Result<AutoStreamingRequestResult, JsValue> {
            let columns = stream_columns_from_metadata(&ids, &revisions, &lengths, &encodings)?;
            let mut pending_columns = Vec::new();
            pending_columns
                .try_reserve_exact(columns.len())
                .map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            for source in &columns {
                if !self.columns.contains_key(&source.id) {
                    return Err(js_err(format!(
                        "resident stream handoff column {:?} is not registered",
                        source.id
                    )));
                }
                pending_columns.push((
                    source.id.clone(),
                    usize::try_from(source.len).map_err(js_err)?,
                    source.revision,
                ));
                next_column_revision = next_column_revision.max(
                    source
                        .revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            let max_primitives_per_chunk = js_safe_u64(
                max_primitives_per_chunk,
                "max_primitives_per_chunk",
            )?;
            let mut source_ids = Vec::new();
            let mut source_revisions = Vec::new();
            source_ids.try_reserve_exact(columns.len()).map_err(js_err)?;
            source_revisions
                .try_reserve_exact(columns.len())
                .map_err(js_err)?;
            source_ids.extend(columns.iter().map(|source| source.id.clone()));
            source_ids.sort_unstable();
            for id in &source_ids {
                source_revisions.push(
                    columns
                        .iter()
                        .find(|source| &source.id == id)
                        .expect("sorted handoff id came from the prepared metadata")
                        .revision as f64,
                );
            }
            let (display_config, _, display_scale) = self.display_config();
            let result = self
                .renderer
                .request_resident_stream_handoff_with_display_scale(
                    self.chart_id,
                    &self.view,
                    display_config,
                    display_scale,
                    columns,
                    StreamingChartOptions {
                        size: self.surface_size,
                        clear_color: self.clear_color,
                        max_primitives_per_chunk,
                    },
                )
                .map_err(js_err)?;
            let (name, pending_latest) = match result {
                renderer::AutoStreamingRequest::Started { .. } => {
                    self.pending_stream_handoff = Some(PendingStreamHandoffMetadata {
                        columns: pending_columns,
                        next_column_revision,
                    });
                    self.request_host_redraw();
                    ("started", false)
                }
                renderer::AutoStreamingRequest::Active { pending_latest, .. } => {
                    source_ids.clear();
                    source_revisions.clear();
                    ("active", pending_latest.is_some())
                }
                renderer::AutoStreamingRequest::Complete { pending_latest, .. } => {
                    source_ids.clear();
                    source_revisions.clear();
                    ("complete", pending_latest.is_some())
                }
            };
            let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
            Ok(AutoStreamingRequestResult {
                status: name,
                revision: status.revision.to_string(),
                job_id: status.job_id.map(|id| id.to_string()),
                submitted_primitives: status.submitted_primitives as f64,
                total_primitives: status.total_primitives as f64,
                source_ids,
                source_revisions,
                pending_latest,
            })
        }

        /// Reserve or inspect the next exact range request. Repeated calls
        /// return the same ranges until `auto_stream_chart_submit_ranges`.
        pub fn auto_stream_chart_request_ranges(
            &mut self,
        ) -> Result<AutoStreamingRangeRequestResult, JsValue> {
            let result = match self
                .renderer
                .auto_stream_chart_request_ranges(self.chart_id)
            {
                Ok(result) => result,
                Err(error) => {
                    self.pending_stream_handoff = None;
                    return Err(js_err(error));
                }
            };
            let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
            let response = AutoStreamingRangeRequestResult::from_core(
                result, counts.revision.to_string(),
                (counts.submitted_primitives as f64, counts.total_primitives as f64),
            )?;
            if response.status == "complete" {
                if let Some(pending) = self.pending_stream_handoff.take() {
                    for (id, len, revision) in pending.columns {
                        self.columns.insert(id.clone(), len);
                        self.column_revisions.insert(id, revision);
                    }
                    self.next_column_revision = pending.next_column_revision;
                    self.retire_series_extent_cache();
                }
            }
            self.request_host_redraw();
            Ok(response)
        }

        /// Independent selected-row request; it never advances the data cursor.
        pub fn stream_selection_request_ranges(&mut self) -> Result<AutoStreamingRangeRequestResult, JsValue> {
            let request = self.renderer.request_stream_selection_ranges(self.chart_id).map_err(js_err)?;
            let revision_name = self.renderer.stream_status(self.chart_id).map_err(js_err)?.desired_revision.to_string();
            let (ticket, _revision, request) = match request {
                renderer::StreamingSelectionRequest::Ready { ticket, revision, ranges } =>
                    (Some(ticket), revision, renderer::AutoStreamingRangeRequest::Ready {
                        revision, ranges, submitted_primitives: 0, total_primitives: 0,
                    }),
                renderer::StreamingSelectionRequest::Backpressure { revision } =>
                    (None, revision, renderer::AutoStreamingRangeRequest::Backpressure {
                        revision, submitted_primitives: 0, total_primitives: 0,
                    }),
                renderer::StreamingSelectionRequest::Complete { revision } =>
                    (None, revision, renderer::AutoStreamingRangeRequest::Complete {
                        revision, pending_latest: None,
                    }),
                renderer::StreamingSelectionRequest::Failed { revision } => {
                    let mut response = AutoStreamingRangeRequestResult::from_core(
                        renderer::AutoStreamingRangeRequest::Complete { revision, pending_latest: None },
                        revision_name, (0.0, 0.0),
                    )?;
                    response.status = "failed";
                    return Ok(response);
                }
            };
            let mut result = AutoStreamingRangeRequestResult::from_core(request, revision_name, (0.0, 0.0))?;
            result.selection_ticket = ticket;
            Ok(result)
        }

        pub fn stream_selection_submit_ranges(
            &mut self, request: &AutoStreamingRangeRequestResult, ids: Vec<String>,
            revisions: Vec<f64>, source_lengths: Vec<f64>, offsets: Vec<f64>, sources: js_sys::Array,
        ) -> Result<(), JsValue> {
            let ticket = request.selection_ticket.ok_or_else(|| js_err("not a selected-row request"))?;
            let sources = js_stream_sources(&sources)?;
            let bindings = stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
            self.renderer.submit_stream_selection_ranges(ticket, &bindings).map_err(js_err)?;
            self.request_host_redraw();
            Ok(())
        }

        pub fn discard_stream_selection_request(&mut self, request: &AutoStreamingRangeRequestResult) -> Result<(), JsValue> {
            let ticket = request.selection_ticket.ok_or_else(|| js_err("not a selected-row request"))?;
            self.renderer.abandon_stream_selection(ticket).map_err(js_err)
        }

        pub fn suspend_stream_selection(&mut self) -> Result<(), JsValue> {
            self.renderer.suspend_stream_selection(self.chart_id).map_err(js_err)
        }

        /// Submit exactly the typed-array chunks named by the pending range
        /// request. The arrays are borrowed for this call only.
        pub fn auto_stream_chart_submit_ranges(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            source_lengths: Vec<f64>,
            offsets: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<StreamingStepResult, JsValue> {
            let sources = js_stream_sources(&sources)?;
            let bindings = stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
            let progress = self
                .renderer
                .auto_stream_chart_submit_ranges(self.chart_id, &bindings)
                .map_err(js_err)?;
            let result = match progress {
                renderer::AutoStreamingProgress::Submitted {
                    submitted_primitives,
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "submitted",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::Backpressure {
                    submitted_primitives,
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "backpressure",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::AllSubmitted {
                    total_primitives,
                    ..
                } => StreamingStepResult {
                    status: "all_submitted",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: total_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::AutoStreamingProgress::Complete { pending_latest, .. } => {
                    let counts = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
                    StreamingStepResult {
                        status: "complete",
                        revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                        submitted_primitives: counts.submitted_primitives as f64,
                        total_primitives: counts.total_primitives as f64,
                        pending_latest: pending_latest.is_some(),
                    }
                }
            };
            self.request_host_redraw();
            Ok(result)
        }

        /// Queue cancellation only when an automatic stream is active.
        pub fn interrupt_render(&mut self) -> Result<String, JsValue> {
            let status = self
                .renderer
                .interrupt_render(self.chart_id)
                .map_err(js_err)?;
            self.pending_stream_handoff = None;
            Ok(match status {
                renderer::RenderInterruptStatus::StreamCancelQueued => {
                    "stream_cancel_queued".to_owned()
                }
                renderer::RenderInterruptStatus::Resident => "resident".to_owned(),
            })
        }

        /// Register N replayable typed-array columns without uploading or
        /// retaining their contents. Each source must be a `Float32Array` or
        /// `Float64Array`; the latter uses the renderer's hi/lo encoding.
        pub fn register_streaming_columns(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<(), JsValue> {
            let sources = js_stream_sources(&sources)?;
            let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
            let mut next_columns = self.columns.clone();
            let mut next_revisions = self.column_revisions.clone();
            next_columns.try_reserve(ids.len()).map_err(js_err)?;
            next_revisions.try_reserve(ids.len()).map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            let mut columns = Vec::new();
            columns.try_reserve_exact(ids.len()).map_err(js_err)?;
            for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
                columns.push(StreamColumn {
                    id: id.clone(),
                    len: source.len() as u64,
                    encoding: source.encoding(),
                    replay: StreamReplay::RandomAccess,
                    revision: *revision,
                    statistics: StreamStatistics::Unknown,
                });
                next_columns.insert(id.clone(), source.len());
                next_revisions.insert(id.clone(), *revision);
                next_column_revision = next_column_revision.max(
                    revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            self.renderer
                .register_streamed_columns(columns)
                .map_err(js_err)?;
            self.columns = next_columns;
            self.column_revisions = next_revisions;
            self.next_column_revision = next_column_revision;
            Ok(())
        }

        /// Atomically publish newer revisions for existing streamed columns.
        /// The renderer still retains no JavaScript typed-array reference.
        pub fn replace_streaming_columns(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<(), JsValue> {
            let sources = js_stream_sources(&sources)?;
            let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
            let mut next_columns = self.columns.clone();
            let mut next_revisions = self.column_revisions.clone();
            next_columns.try_reserve(ids.len()).map_err(js_err)?;
            next_revisions.try_reserve(ids.len()).map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            let mut columns = Vec::new();
            columns.try_reserve_exact(ids.len()).map_err(js_err)?;
            for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
                columns.push(StreamColumn {
                    id: id.clone(),
                    len: source.len() as u64,
                    encoding: source.encoding(),
                    replay: StreamReplay::RandomAccess,
                    revision: *revision,
                    statistics: StreamStatistics::Unknown,
                });
                next_columns.insert(id.clone(), source.len());
                next_revisions.insert(id.clone(), *revision);
                next_column_revision = next_column_revision.max(
                    revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            self.renderer.replace_streamed_columns(columns).map_err(js_err)?;
            self.columns = next_columns;
            self.column_revisions = next_revisions;
            self.next_column_revision = next_column_revision;
            self.retire_series_extent_cache();
            Ok(())
        }

        /// Normalize a complete referenced-column closure from resident or
        /// mixed authority to exact streamed metadata. The typed arrays are
        /// borrowed only for the atomic transition and are not retained by
        /// Rust; the high-level JS executor keeps the replayable references.
        pub fn demote_auto_resident_columns(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<(), JsValue> {
            let sources = js_stream_sources(&sources)?;
            let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
            let mut next_columns = self.columns.clone();
            let mut next_revisions = self.column_revisions.clone();
            next_columns.try_reserve(ids.len()).map_err(js_err)?;
            next_revisions.try_reserve(ids.len()).map_err(js_err)?;
            let mut next_column_revision = self.next_column_revision;
            let mut columns = Vec::new();
            let mut bindings = Vec::new();
            columns.try_reserve_exact(ids.len()).map_err(js_err)?;
            bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
            for ((id, revision), source) in ids.iter().zip(&revisions).zip(&sources) {
                columns.push(StreamColumn {
                    id: id.clone(),
                    len: source.len() as u64,
                    encoding: source.encoding(),
                    replay: StreamReplay::RandomAccess,
                    revision: *revision,
                    statistics: StreamStatistics::Unknown,
                });
                bindings.push(StreamSourceBinding {
                    id,
                    revision: *revision,
                    source: source.source(),
                });
                next_columns.insert(id.clone(), source.len());
                next_revisions.insert(id.clone(), *revision);
                next_column_revision = next_column_revision.max(
                    revision
                        .checked_add(1)
                        .ok_or_else(|| js_err("column revision counter exhausted"))?,
                );
            }
            let mut changed_ids = Vec::new();
            changed_ids.try_reserve_exact(ids.len()).map_err(js_err)?;
            changed_ids.extend(ids.iter().map(String::as_str));
            self.renderer
                .demote_resident_columns_to_streamed(&changed_ids, columns, &bindings)
                .map_err(js_err)?;
            self.columns = next_columns;
            self.column_revisions = next_revisions;
            self.next_column_revision = next_column_revision;
            self.retire_series_extent_cache();
            self.request_host_redraw();
            Ok(())
        }

        /// Start or restart this chart's exact streaming surface and renderer-
        /// owned CPU cursor. No LOD or downsampling is performed.
        pub fn begin_streaming(&mut self, max_primitives_per_chunk: f64) -> Result<(), JsValue> {
            let max_primitives_per_chunk = js_safe_u64(
                max_primitives_per_chunk,
                "max_primitives_per_chunk",
            )?;
            let (display_config, _, _) = self.display_config();
            let display_chart = Chart::new(display_config);
            self.renderer
                .update_transform(&self.view, &display_chart)
                .map_err(js_err)?;
            self.renderer
                .begin_streaming_chart(
                    self.chart_id,
                    &self.view,
                    StreamingChartOptions {
                        size: self.surface_size,
                        clear_color: self.clear_color,
                        max_primitives_per_chunk,
                    },
                )
                .map_err(js_err)?;
            self.request_host_redraw();
            Ok(())
        }

        /// Advance at most one renderer-selected chunk. `sources` contains the
        /// original replayable typed arrays; only the requested range is read
        /// directly into mapped staging, and no owned CPU data mirror is made.
        pub fn stream_step(
            &mut self,
            ids: Vec<String>,
            revisions: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<StreamingStepResult, JsValue> {
            let sources = js_stream_sources(&sources)?;
            let revisions = validate_stream_bindings(&ids, &revisions, &sources)?;
            let mut bindings = Vec::new();
            bindings.try_reserve_exact(ids.len()).map_err(js_err)?;
            for ((id, revision), source) in ids.iter().zip(revisions).zip(&sources) {
                bindings.push(StreamSourceBinding {
                    id,
                    revision,
                    source: source.source(),
                });
            }
            let progress = self
                .renderer
                .stream_chart_step(self.chart_id, &self.view, &bindings)
                .map_err(js_err)?;
            let result = match progress {
                renderer::StreamingProgress::Submitted {
                    submitted_primitives,
                    total_primitives,
                } => {
                    self.request_host_redraw();
                    StreamingStepResult {
                        status: "submitted",
                        revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                        submitted_primitives: submitted_primitives as f64,
                        total_primitives: total_primitives as f64,
                        pending_latest: false,
                    }
                }
                renderer::StreamingProgress::Backpressure {
                    submitted_primitives,
                    total_primitives,
                } => StreamingStepResult {
                    status: "backpressure",
                    revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                    submitted_primitives: submitted_primitives as f64,
                    total_primitives: total_primitives as f64,
                    pending_latest: false,
                },
                renderer::StreamingProgress::AllSubmitted { total_primitives } => {
                    StreamingStepResult {
                        status: "all_submitted",
                        revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                        submitted_primitives: total_primitives as f64,
                        total_primitives: total_primitives as f64,
                        pending_latest: false,
                    }
                }
            };
            Ok(result)
        }

        pub fn cancel_streaming(&mut self) -> Result<(), JsValue> {
            self.renderer
                .cancel_streaming_chart(self.chart_id)
                .map_err(js_err)?;
            Ok(())
        }

        /// Cancel this chart and resolve only after its submitted work and
        /// retired renderer-owned surfaces have crossed the completion fence.
        pub async fn cancel_streaming_and_wait(&mut self) -> Result<(), JsValue> {
            self.pending_stream_handoff = None;
            self.renderer.cancel_streaming_chart_and_wait(self.chart_id).await.map_err(js_err)
        }

        pub fn set_stream_chunk_budget(&mut self, primitives: f64) -> Result<(), JsValue> {
            let primitives = js_safe_u64(primitives, "stream chunk budget")?;
            self.renderer.set_stream_chunk_budget(self.chart_id, primitives).map_err(js_err)
        }

        /// Wake after work already submitted to this queue completes. The
        /// returned promise owns only its completion receiver, not a chart or
        /// renderer borrow. Resolving does not service receipts or draw a frame.
        pub fn streaming_gpu_ready(&self) -> js_sys::Promise {
            let (sender, receiver) = futures_channel::oneshot::channel();
            self.renderer.queue().on_submitted_work_done(move || {
                let _ = sender.send(());
            });
            wasm_bindgen_futures::future_to_promise(async move {
                receiver.await.map_err(|_| js_err("stream GPU completion callback was dropped"))?;
                Ok(JsValue::UNDEFINED)
            })
        }

        /// Pure metadata inspection; does not poll, reserve a range, or draw.
        pub fn stream_status(&self) -> Result<String, JsValue> {
            let status = self.renderer.stream_status(self.chart_id).map_err(js_err)?;
            let residency = self.renderer.view_residency_status(self.chart_id).map_err(js_err)?;
            let name = match status.status {
                renderer::StreamingState::Idle => "idle",
                renderer::StreamingState::Active => "active",
                renderer::StreamingState::AllSubmitted => "all_submitted",
                renderer::StreamingState::Complete => "complete",
                renderer::StreamingState::Cancelling => "cancelling",
                renderer::StreamingState::Cancelled => "cancelled",
            };
            Ok(serde_json::json!({
                "status": name,
                "revision": status.revision.to_string(),
                "desired_revision": status.desired_revision.to_string(),
                "published_revision": status.published_revision.map(|value| value.to_string()),
                "job_id": status.job_id.map(|value| value.to_string()),
                "submitted_primitives": status.submitted_primitives,
                "total_primitives": status.total_primitives,
                "in_flight_chunks": status.in_flight_chunks,
                "reserved_gpu_bytes": status.reserved_gpu_bytes,
                "auto_fit_pending": status.auto_fit_pending,
                "view_residency": {
                    "state": residency.state,
                    "needed_bytes": residency.needed_bytes,
                    "refusal_reason": residency.refusal_reason,
                    "picking_available": residency.picking_available,
                },
            }).to_string())
        }

        /// Metadata-only admission for incoming columns; no payload is read,
        /// registered, uploaded, or reserved. Chart-derived buffers are excluded.
        pub fn inspect_column_admission(&self, lengths: Vec<f64>) -> Result<String, JsValue> {
            let lengths = lengths.into_iter().map(|value| js_safe_u64(value, "column length"))
                .collect::<Result<Vec<_>, _>>()?;
            let report = self.renderer.inspect_column_admission(&lengths);
            Ok(serde_json::json!({
                "admissible": report.is_admissible(),
                "reason": resident_admission_reason(report.status),
                "resident_column_bytes": report.resident_column_bytes,
                "resident_working_set_bytes": report.resident_working_set_bytes,
                "transition_peak_bytes": report.transition_peak_bytes,
                "current_gpu_bytes": report.current_gpu_bytes,
                "memory_budget_bytes": report.memory_budget_bytes,
                "working_set_limit_bytes": report.working_set_limit_bytes,
                "device_buffer_limit_bytes": report.device_buffer_limit_bytes,
                "includes_chart_derived_resources": false,
                "reservation": false,
            }).to_string())
        }

        pub fn streaming_capabilities(&self) -> String {
            let supported = self.renderer.inspect_streaming_capability(self.chart_id);
            let render_supported = supported.is_ok();
            serde_json::json!({
                "render_supported": render_supported,
                "reason": supported.err().map(|error| error.to_string()),
                "range_sources": true,
                "adaptive_chunk_budget": true,
                "async_cancel": true,
                "stream_pick": false,
                "stream_export": render_supported,
                "operations_require_completed_revision": true,
                "contour": false,
            }).to_string()
        }

        /// Synchronous fit request routing, allowing the facade to wait for
        /// renderer progress without holding a wasm mutable borrow.
        pub fn request_stream_auto_fit(&mut self, padding: f64) -> Result<bool, JsValue> {
            let streamed = self.chart_series().iter().any(|series| {
                referenced_columns(series).into_iter().any(|id| matches!(
                    self.renderer.logical_column(id), Some(renderer::LogicalColumn::Streamed(_))
                ))
            });
            if streamed {
                self.renderer.request_stream_auto_fit(self.chart_id, padding).map_err(js_err)?;
            }
            Ok(streamed)
        }

        pub fn streaming_usage(&self) -> StreamingUsageResult {
            let usage = self.renderer.streaming_usage();
            StreamingUsageResult {
                active_charts: usage.active_charts as u32,
                in_flight_chunks: usage.in_flight_chunks as u32,
                reserved_gpu_bytes: usage.reserved_gpu_bytes as f64,
            }
        }

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
        pub fn update_register_column_f32(
            &mut self,
            id: &str,
            data: &[f32],
        ) -> Result<(), JsValue> {
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
        pub fn update_register_column_f64(
            &mut self,
            id: &str,
            data: &[f64],
        ) -> Result<(), JsValue> {
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

        // ---- series registry (id-keyed upsert / unregister) ----

        /// Register or update a line series over two registered columns.
        ///
        /// Upsert by `series_id`: a new id takes the next color of the active
        /// cycle; an existing id is replaced in place and keeps its color.
        /// Non-empty `label` adds/updates the legend row.
        pub fn add_line_series(
            &mut self,
            series_id: &str,
            x_column: &str,
            y_column: &str,
            line_width: f32,
            label: &str,
        ) -> Result<(), JsValue> {
            let current = self.chart_series().to_vec();
            let existing = current.iter().position(|c| c.series_id == series_id);
            let color = match existing {
                Some(i) => match &current[i].render_type {
                    DataRenderType::Line { line } => line.line_color,
                    _ => self.cycle.color(i),
                },
                None => self.cycle.color(self.color_seq),
            };
            let next_color_seq = if existing.is_none() {
                self.color_seq
                    .checked_add(1)
                    .ok_or_else(|| js_err("series color sequence exhausted"))?
            } else {
                self.color_seq
            };
            let label_changed = !label.is_empty();
            let rich_label = if label_changed {
                Some(self.label_text(label))
            } else {
                existing.and_then(|i| current[i].label.clone())
            };
            let plain_label = if label_changed {
                Some(label.to_string())
            } else {
                existing.and_then(|i| self.labels[i].clone())
            };

            let cfg = SeriesConfig {
                series_id: series_id.into(),
                source_id: None,
                label: rich_label.clone(),
                x_column: x_column.into(),
                y_column: y_column.into(),
                render_type: DataRenderType::Line {
                    line: DataLineStyleConfig {
                        line_style: LineStylePreset::Solid,
                        line_color: color,
                        line_width: line_width.max(0.5),
                    },
                },
            };
            self.ensure_columns_exist(&cfg)?;
            let (scale, _) = self.display_scale_and_panel();
            let mut proposed = current;
            if let Some(index) = existing {
                proposed[index] = cfg.clone();
            } else {
                proposed.try_reserve(1).map_err(js_err)?;
                proposed.push(cfg.clone());
            }
            let prepared_extents = if self.renderer.errorbar_extent_engine_ready() {
                Some(self.prepare_series_extent_cache(&proposed, &self.column_revisions, false)?)
            } else {
                None
            };
            let mut next_styles = Vec::new();
            next_styles.try_reserve(proposed.len()).map_err(js_err)?;
            for series in &proposed {
                next_styles.push(self.renderer.create_style_for_series_scaled(series, scale));
            }
            let mut next_labels = self.labels.clone();
            if let Some(index) = existing {
                next_labels[index] = plain_label;
            } else {
                next_labels.try_reserve(1).map_err(js_err)?;
                next_labels.push(plain_label);
            }
            let mut next_config = self.chart_config().clone();
            if label_changed && let Some(label) = rich_label {
                match existing {
                    Some(index) => renderer::config::set_legend_entry_label(
                        &mut next_config.legend.content,
                        index,
                        renderer::config::series_symbol_segments(&cfg),
                        label.segments,
                    ),
                    None => renderer::config::append_legend_entry_rich(
                        &mut next_config.legend.content,
                        renderer::config::series_symbol_segments(&cfg),
                        label.segments,
                    ),
                }
                next_config.legend.visible = true;
            } else {
                renderer::config::update_legend_symbols_preserving_text(
                    &mut next_config.legend.content,
                    &proposed,
                );
            }

            self.replace_chart_state(next_config, proposed)?;
            self.styles = next_styles;
            self.labels = next_labels;
            self.color_seq = next_color_seq;
            if let Some(prepared_extents) = prepared_extents {
                self.publish_series_extent_cache(prepared_extents);
            }
            Ok(())
        }

        /// Set / change / remove a series' legend label. `'\n'` breaks lines;
        /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments.
        /// Empty string removes the legend row. Returns `true` when the
        /// series exists.
        pub fn set_series_label(&mut self, series_id: &str, label: &str) -> Result<bool, JsValue> {
            let mut series = self.chart_series().to_vec();
            let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
                return Ok(false);
            };
            let mut config = self.chart_config().clone();
            let mut labels = self.labels.clone();
            if label.is_empty() {
                labels[i] = None;
                series[i].label = None;
                if renderer::config::remove_legend_entry(&mut config.legend.content, i)
                    && config.legend.content.segments.is_empty()
                {
                    config.legend.visible = false;
                }
            } else {
                let label = self.label_text(label);
                labels[i] = Some(Self::rich_text_to_plain(&label));
                series[i].label = Some(label.clone());
                renderer::config::set_legend_entry_label(
                    &mut config.legend.content,
                    i,
                    renderer::config::series_symbol_segments(&series[i]),
                    label.segments,
                );
                config.legend.visible = true;
            }
            self.replace_chart_state(config, series)?;
            self.labels = labels;
            Ok(true)
        }

        /// Unregister a series. Columns stay registered. Auto-managed legends
        /// remove its row; freely edited legends preserve their text and only
        /// synchronize recognized series symbols.
        /// Returns `true` when the series existed.
        pub fn remove_series(&mut self, series_id: &str) -> Result<bool, JsValue> {
            let mut series = self.chart_series().to_vec();
            let Some(i) = series.iter().position(|c| c.series_id == series_id) else {
                return Ok(false);
            };
            series.remove(i);
            let mut config = self.chart_config().clone();
            if self.legend_auto_managed {
                if renderer::config::remove_legend_entry(&mut config.legend.content, i)
                    && config.legend.content.segments.is_empty()
                {
                    config.legend.visible = false;
                }
            } else {
                renderer::config::update_legend_symbols_preserving_text(
                    &mut config.legend.content,
                    &series,
                );
            }
            self.replace_chart_state(config, series)?;
            self.styles.remove(i);
            self.labels.remove(i);
            self.retire_series_extent_cache();
            Ok(true)
        }

        /// Fit the x axis to a column's range with proportional padding.
        pub fn auto_fit_x(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            let mut chart = Chart::new(self.chart_config().clone());
            chart
                .auto_fit_x(self.renderer.pool(), column, padding)
                .map_err(js_err)?;
            self.replace_chart_config(chart.config().clone())?;
            Ok(())
        }

        pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            let mut chart = Chart::new(self.chart_config().clone());
            chart
                .auto_fit_y(self.renderer.pool(), column, padding)
                .map_err(js_err)?;
            self.replace_chart_config(chart.config().clone())?;
            Ok(())
        }

        /// Fit BOTH axes to the union of every registered series, leaving a
        /// uniform `padding` fraction of the data span as margin on each
        /// side (`0.0` = no padding, `0.05` = 5% top/bottom/left/right).
        /// This is the whole fit policy — no rounding of the range ends;
        /// ticks land on nice values inside the range by themselves. Hosts
        /// should call this instead of re-deriving ranges.
        ///
        /// Each series contributes its exact original GPU primitive domain:
        /// valid adjacent segments for line-only mode, paired finite points
        /// for point-bearing modes, and enabled error endpoints. The reduction
        /// is compiled on first fit (`createComputePipelineAsync` on wasm)
        /// and cached by the normalized mode plus role-specific column
        /// revisions. Matrix-backed series use the same engine to return the
        /// exact cell/sample-lattice bounds; only histograms remain a metadata
        /// union because their uploaded edge column already is their geometry.
        /// Series registration itself does not submit GPU work.
        pub async fn auto_fit_all(&mut self, padding: f64) -> Result<(), JsValue> {
            if self.request_stream_auto_fit(padding)? {
                self.request_host_redraw();
                return Ok(());
            }
            let series = self.chart_series().to_vec();
            if series
                .iter()
                .any(|cfg| series_extent_mode(&cfg.render_type).is_some())
            {
                self.renderer
                    .ensure_errorbar_extent_engine()
                    .await
                    .map_err(js_err)?;
            }
            let token = self
                .renderer
                .begin_fit_commit(self.chart_id)
                .map_err(js_err)?;
            let prepared =
                self.prepare_series_extent_cache(&series, &self.column_revisions, true)?;
            let mut jobs = Vec::new();
            jobs.try_reserve(prepared.extents.len()).map_err(js_err)?;
            jobs.extend(prepared.extents.values().cloned());
            self.publish_series_extent_cache(prepared);

            let mut x_ext = FitExtent::EMPTY;
            let mut y_ext = FitExtent::EMPTY;
            for job in jobs {
                let extent = job.wait().await.map_err(js_err)?;
                if let Some(extent) = extent {
                    x_ext.union(&extent.x);
                    y_ext.union(&extent.y);
                }
            }
            let scalar_extent = |value: f64| FitExtent {
                min: value,
                max: value,
                min_positive: (value > 0.0).then_some(value),
            };
            for cfg in &series {
                match &cfg.render_type {
                    DataRenderType::Histogram { bar } => {
                        let (edge_column, count_column) =
                            crate::histogram_fit_columns(cfg, &bar.orientation);
                        let edge = Chart::slot_extent(self.renderer.pool(), edge_column)
                            .map_err(js_err)?;
                        let mut count = Chart::slot_extent(self.renderer.pool(), count_column)
                            .map_err(js_err)?;
                        count.union(&scalar_extent(bar.baseline));
                        if matches!(
                            &bar.orientation,
                            renderer::data_config::BarOrientation::Vertical
                        ) {
                            x_ext.union(&edge);
                            y_ext.union(&count);
                        } else {
                            x_ext.union(&count);
                            y_ext.union(&edge);
                        }
                    }
                    _ => {}
                }
            }
            if !self.alive.get() {
                return Err(js_err("chart was dropped while auto_fit_all was pending"));
            }
            self.renderer
                .commit_auto_fit_all_if_current(&token, &x_ext, &y_ext, padding)
                .map_err(js_err)?;
            self.request_host_redraw();
            Ok(())
        }

        /// Fit the shared z axis to every matrix value column in the chart.
        /// A chart without a colourbar is left unchanged.
        pub fn auto_fit_colorbar(&mut self, padding: f64) -> Result<(), JsValue> {
            let mut z_extent = FitExtent::EMPTY;
            for cfg in self.chart_series() {
                let matrix = match &cfg.render_type {
                    DataRenderType::Heatmap { matrix, .. }
                    | DataRenderType::Contour { matrix, .. }
                    | DataRenderType::HeatmapContour { matrix, .. } => matrix,
                    _ => continue,
                };
                for id in &matrix.columns {
                    z_extent.union(&Chart::slot_extent(self.renderer.pool(), id).map_err(js_err)?);
                }
            }
            let mut chart = Chart::new(self.chart_config().clone());
            chart.auto_fit_colorbar_extent(&z_extent, padding);
            self.replace_chart_config(chart.config().clone())?;
            self.request_host_redraw();
            Ok(())
        }

        /// Replace one contour series' levels with axis-native nice levels.
        /// The resulting explicit values remain the series SSoT.
        pub fn set_contour_nice_levels(
            &mut self,
            series_id: &str,
            target_count: usize,
            use_colormap_colors: bool,
        ) -> Result<u32, JsValue> {
            let colorbar = self
                .chart_config()
                .colorbar
                .clone()
                .ok_or_else(|| js_err("contour levels need Config.colorbar"))?;
            let mut series = self.chart_series().to_vec();
            let cfg = series
                .iter_mut()
                .find(|cfg| cfg.series_id == series_id)
                .ok_or_else(|| js_err("no series with this id in the chart"))?;
            let contour = match &mut cfg.render_type {
                DataRenderType::Contour { contour, .. }
                | DataRenderType::HeatmapContour { contour, .. } => contour,
                _ => return Err(js_err("series does not have contour levels")),
            };
            contour
                .set_nice_levels(&colorbar.axis, target_count)
                .map_err(|error| js_err(format!("{error:?}")))?;
            if use_colormap_colors {
                contour.set_colormap_colors(&colorbar);
            }
            let count = u32::try_from(contour.levels.len()).map_err(js_err)?;
            let json = serde_json::to_string(&series).map_err(js_err)?;
            self.set_series(&json)?;
            self.request_host_redraw();
            Ok(count)
        }

        /// Report the effective primitive/grid extent currently drawn.
        pub fn series_draw_info(&self, series_id: &str) -> Result<String, JsValue> {
            let info = self
                .renderer
                .series_draw_info(self.chart_id, series_id)
                .map_err(js_err)?;
            serde_json::to_string(&serde_json::json!({
                "drawn_count": info.drawn_count,
                "cols": info.cols,
                "rows": info.rows,
                "truncated": info.truncated,
            }))
            .map_err(js_err)
        }

        // ---- titles ----

        pub fn set_title(&mut self, text: &str) -> Result<(), JsValue> {
            let mut config = self.chart_config().clone();
            config.chart_title.text.segments = rich_segments_from_text(text);
            self.replace_chart_config(config)
        }

        pub fn set_x_title(&mut self, text: &str) -> Result<(), JsValue> {
            let mut config = self.chart_config().clone();
            config.bottom_x.title_option.text.segments = rich_segments_from_text(text);
            self.replace_chart_config(config)
        }

        pub fn set_y_title(&mut self, text: &str) -> Result<(), JsValue> {
            let mut config = self.chart_config().clone();
            config.left_y.title_option.text.segments = rich_segments_from_text(text);
            self.replace_chart_config(config)
        }

        /// Set the colourbar title text and make it visible. An empty string
        /// hides it. The full rich-text/title offsets remain available through
        /// `set_colorbar_axis` or the chart Config SSoT.
        pub fn set_colorbar_title(&mut self, text: &str) -> Result<(), JsValue> {
            let mut config = self.chart_config().clone();
            let bar = config
                .colorbar
                .as_mut()
                .ok_or_else(|| js_err("colorbar title needs Config.colorbar"))?;
            bar.axis.title_option.text.segments = rich_segments_from_text(text);
            bar.axis.title_option.visible = !text.is_empty();
            self.replace_chart_config(config)
        }

        /// Replace the colourbar's z-axis options as one SSoT value. This is
        /// the focused edit boundary for tick direction/style/length, value
        /// direction, tick-label style/offsets, and title options.
        pub fn set_colorbar_axis(&mut self, json: &str) -> Result<(), JsValue> {
            let axis: renderer::config::AxisOptions = serde_json::from_str(json).map_err(js_err)?;
            let mut config = self.chart_config().clone();
            let bar = config
                .colorbar
                .as_mut()
                .ok_or_else(|| js_err("colorbar axis needs Config.colorbar"))?;
            bar.axis = axis;
            self.replace_chart_config(config)
        }

        // ---- presets ----

        /// Apply an axis frame preset to all four axes (decoration-only).
        pub fn apply_axis_preset(&mut self, preset: AxisPreset) -> Result<(), JsValue> {
            let p: renderer::AxisPreset = preset.into();
            let mut config = self.chart_config().clone();
            config.apply_axis_preset(p);
            self.replace_chart_config(config)
        }

        /// Switch the series color rotation: recolors every series in order,
        /// rebuilds their GPU styles, and keeps legend swatches in sync.
        pub fn apply_color_cycle(&mut self, cycle: ColorCycle) -> Result<(), JsValue> {
            let cycle = renderer::ColorCycle::from(cycle);
            let mut series = self.chart_series().to_vec();
            for (i, cfg) in series.iter_mut().enumerate() {
                cycle.apply_to_series(cfg, i);
            }
            let (scale, _) = self.display_scale_and_panel();
            let mut styles = Vec::new();
            styles.try_reserve(series.len()).map_err(js_err)?;
            for cfg in &series {
                styles.push(self.renderer.create_style_for_series_scaled(cfg, scale));
            }
            let mut config = self.chart_config().clone();
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &series,
            );
            self.replace_chart_state(config, series)?;
            self.cycle = cycle;
            self.color_seq = styles.len();
            self.styles = styles;
            Ok(())
        }

        // ---- SSoT I/O ----
        //
        // The whole option tree (`Config`) is plain data; these round-trip it
        // as JSON so a host can read it, edit anything — axis scale, tick
        // shape/length, colors, fonts, label text — and hand it back. The
        // standard flow: auto-fit first, then refine via the SSoT.
        // Full schema reference: crates/web/SCHEMA.md.

        /// Serialize the full chart option SSoT to a JSON string.
        /// JS: `const cfg = JSON.parse(chart.get_config());`
        pub fn get_config(&self) -> Result<String, JsValue> {
            serde_json::to_string(self.chart_config()).map_err(js_err)
        }

        /// Replace the whole option SSoT from JSON. Marks everything dirty —
        /// the next `frame()` re-rasters the chrome and refreshes the
        /// transform, exactly like any other config edit.
        /// JS: `chart.set_config(JSON.stringify(cfg));`
        pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
            let new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
            let old_config = self.chart_config();
            let legend_content_changed = old_config.legend.content != new_cfg.legend.content;
            let series = self.chart_series().to_vec();
            let (_, _, scale) = renderer::display_config_for_surface(&new_cfg, self.surface_size);
            let mut new_styles = Vec::new();
            new_styles.try_reserve(series.len()).map_err(js_err)?;
            for cfg in &series {
                new_styles.push(self.renderer.create_style_for_series_scaled(cfg, scale));
            }
            self.replace_chart_config(new_cfg)?;
            if legend_content_changed {
                self.legend_auto_managed = false;
            }
            self.styles = new_styles;
            Ok(())
        }

        /// Serialize the series declarations (columns, render type, styles).
        pub fn get_series(&self) -> Result<String, JsValue> {
            serde_json::to_string(self.chart_series()).map_err(js_err)
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
            let old_series = self.chart_series().to_vec();
            let mut new_labels = Vec::new();
            new_labels.try_reserve(new_series.len()).map_err(js_err)?;
            for cfg in &new_series {
                new_labels.push(
                    cfg.label
                        .as_ref()
                        .map(Self::rich_text_to_plain)
                        .or_else(|| {
                            old_series
                                .iter()
                                .position(|old| old.series_id == cfg.series_id)
                                .and_then(|i| self.labels[i].clone())
                        }),
                );
            }
            let (scale, _) = self.display_scale_and_panel();
            let mut new_styles = Vec::new();
            new_styles.try_reserve(new_series.len()).map_err(js_err)?;
            for cfg in &new_series {
                new_styles.push(self.renderer.create_style_for_series_scaled(cfg, scale));
            }
            let mut config = self.chart_config().clone();
            if self.legend_auto_managed {
                let existing = renderer::config::legend_entry_count(&config.legend.content);
                renderer::config::update_legend_symbols_preserving_text(
                    &mut config.legend.content,
                    &new_series,
                );
                if existing > new_series.len() {
                    for index in (new_series.len()..existing).rev() {
                        renderer::config::remove_legend_entry(&mut config.legend.content, index);
                    }
                }
                for index in existing..new_series.len() {
                    let label = new_series[index].label.clone().or_else(|| {
                        new_labels[index]
                            .as_ref()
                            .map(|label| self.label_text(label))
                    });
                    if let Some(label) = label {
                        renderer::config::append_legend_entry_rich(
                            &mut config.legend.content,
                            renderer::config::series_symbol_segments(&new_series[index]),
                            label.segments,
                        );
                    }
                }
                config.legend.visible = !config.legend.content.segments.is_empty();
            }
            let new_len = new_series.len();
            self.replace_chart_state(config, new_series)?;
            self.labels = new_labels;
            self.styles = new_styles;
            self.color_seq = new_len.max(self.color_seq);
            let extent_series = self.chart_series().to_vec();
            let extent_revisions = self.column_revisions.clone();
            self.publish_series_extents_if_engine_ready(&extent_series, &extent_revisions, false)?;
            Ok(())
        }

        /// Explicitly rebuild the auto legend from `SeriesConfig.label`.
        /// Legacy string labels are used only when a series has no rich label.
        pub fn reset_legend_from_series_labels(&mut self) -> Result<(), JsValue> {
            self.rebuild_legend_from_series_labels()
        }

        // ---- pointer interaction (coordinates in canvas pixels) ----

        /// Hit-test the chart chrome at canvas pixel `(x, y)` — returns the
        /// topmost element's stable id (`"data_area"`, `"axis_bottom"`,
        /// `"tick_labels_left"`, `"axis_title_left"`, `"colorbar"`,
        /// `"colorbar_axis"`, `"colorbar_tick_labels"`,
        /// `"colorbar_title"`, …) or `null`. Pure geometry, no selection state
        /// change: the renderer's own layout answers, so hosts don't have to
        /// re-derive box positions for hover cursors / context UI.
        pub fn hit_test(&self, x: f32, y: f32) -> Option<String> {
            let (display_config, _, _) = self.display_config();
            self.hitmap
                .hit_test(
                    &display_config,
                    &CpuTextMeasure::for_style(&display_config.draw_style),
                    x,
                    y,
                )
                .and_then(|id| self.hitmap.get(id))
                .map(|el| el.element_id())
        }

        async fn prepare_gpu_picking(&mut self) -> Result<(), JsValue> {
            self.renderer
                .enable_gpu_picking_async()
                .await
                .map_err(js_err)?;
            self.renderer
                .prepare_gpu_picking_for_chart(self.chart_id)
                .map_err(js_err)
        }

        /// Compile the exact GPU picker and prepare it for the current chart.
        /// Repeated calls, including retries, reuse renderer-owned state.
        pub async fn prewarm_gpu_picking(&mut self) -> Result<(), JsValue> {
            self.prepare_gpu_picking().await
        }

        /// Pick the nearest visible data primitive to canvas pixel `(x, y)`.
        /// Scatter hits use the visible marker size, including per-point style
        /// mapping; line strokes snap to the nearest endpoint data point on
        /// the hit segment. Errorbar stems/caps are not pick targets.
        /// Resolves to a JSON string
        /// `{ source_id, series_id, point_index, distance_px }`, or
        /// `undefined` when no visible primitive is within `max_distance_px`.
        /// The `<figgy-chart>` facade parses the string and normalizes
        /// `undefined` to `null`.
        pub async fn pick_point(
            &mut self,
            x: f32,
            y: f32,
            max_distance_px: f32,
        ) -> Result<JsValue, JsValue> {
            if self.renderer.is_streaming_chart(self.chart_id) {
                let Some(picked) = self.renderer
                    .pick_chart_view_cache_at(self.chart_id, [x, y], max_distance_px)
                    .await.map_err(js_err)? else { return Ok(JsValue::UNDEFINED); };
                let json = crate::picked_point_json_string(&picked).map_err(js_err)?;
                return Ok(JsValue::from_str(&json));
            }
            self.prepare_gpu_picking().await?;
            let ticket = self
                .renderer
                .pick_chart_at(self.chart_id, [x, y], max_distance_px)
                .map_err(js_err)?;
            let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
                return Ok(JsValue::UNDEFINED);
            };
            let json = crate::picked_point_json_string(&picked).map_err(js_err)?;
            Ok(JsValue::from_str(&json))
        }

        /// Move among source rows represented in the completed packed view.
        /// Returns only the original source index; no source or GPU readback.
        pub fn next_view_point_index(
            &self,
            source_id: Option<String>,
            series_id: String,
            current: f64,
            forward: bool,
        ) -> Result<JsValue, JsValue> {
            let current = js_safe_u64(current, "current point index")?;
            let current = usize::try_from(current).map_err(|_| js_err("point index exceeds platform size"))?;
            Ok(self.renderer.next_view_point_index(
                self.chart_id, source_id.as_deref(), &series_id, current, forward,
            ).map_or(JsValue::UNDEFINED, |index| JsValue::from_f64(index as f64)))
        }

        /// Pick the nearest visible data primitive at canvas pixel `(x, y)`;
        /// equal-distance hits follow chart paint order.
        /// Unlike `pick_point`, this includes histogram bins, canonical heatmap
        /// cells and contour levels. The tagged result contains only stable
        /// source/series indices plus `distance_px`; values and geometry stay
        /// in the renderer's GPU-backed SSoT. Resolves to `undefined` on miss.
        pub async fn pick_data(
            &mut self,
            x: f32,
            y: f32,
            max_distance_px: f32,
        ) -> Result<JsValue, JsValue> {
            if self.renderer.is_streaming_chart(self.chart_id) {
                let Some(picked) = self.renderer
                    .pick_chart_view_cache_at(self.chart_id, [x, y], max_distance_px)
                    .await.map_err(js_err)? else { return Ok(JsValue::UNDEFINED); };
                let tagged = renderer::PickedData::Point {
                    source_id: picked.source_id,
                    series_id: picked.series_id,
                    point_index: picked.point_index,
                    distance_px: picked.distance_px,
                };
                let json = crate::picked_data_json_string(&tagged).map_err(js_err)?;
                return Ok(JsValue::from_str(&json));
            }
            self.prepare_gpu_picking().await?;
            let ticket = self
                .renderer
                .pick_chart_data_at(self.chart_id, [x, y], max_distance_px)
                .map_err(js_err)?;
            let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
                return Ok(JsValue::UNDEFINED);
            };
            let json = crate::picked_data_json_string(&picked).map_err(js_err)?;
            Ok(JsValue::from_str(&json))
        }

        /// Replace the picked-point overlay config. Passing JSON `null`
        /// clears it.
        pub fn set_picked_points(&mut self, json: &str) -> Result<(), JsValue> {
            let picked: Option<renderer::config::PickedPointsConfig> =
                serde_json::from_str(json).map_err(js_err)?;
            let mut config = self.chart_config().clone();
            config.picked_points = picked;
            self.replace_chart_config(config)
        }

        /// Replace the typed data-selection overlay config. Passing JSON
        /// `null` clears it. This is independent of the legacy point-only
        /// overlay so existing hosts keep their current behavior.
        pub fn set_picked_data(&mut self, json: &str) -> Result<(), JsValue> {
            let picked: Option<renderer::DataSelectionsConfig> =
                serde_json::from_str(json).map_err(js_err)?;
            let mut config = self.chart_config().clone();
            config.picked_data = picked;
            self.replace_chart_config(config)
        }

        /// Pointer press. Returns `true` while something is selected.
        /// The host can mirror that state in its own UI.
        pub fn on_press(&mut self, x: f32, y: f32) -> Result<bool, JsValue> {
            let (display_config, _, _) = self.display_config();
            let selected = self.chart_selection();
            // Resize handles on the selected element win over hit-testing.
            if let Some(id) = selected
                && let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                && let Some(handle) = rz.hit_resize_handle(
                    &display_config,
                    &CpuTextMeasure::for_style(&display_config.draw_style),
                    x,
                    y,
                )
            {
                self.resizing = Some(handle);
                self.dragging = false;
                return Ok(true);
            }

            let new_sel = self.hitmap.hit_test(
                &display_config,
                &CpuTextMeasure::for_style(&display_config.draw_style),
                x,
                y,
            );
            let dragging = new_sel.is_some_and(|id| {
                self.hitmap
                    .get(id)
                    .is_some_and(|el| el.as_draggable().is_some())
            });
            if new_sel != selected {
                self.renderer
                    .set_chart_selection(self.chart_id, new_sel)
                    .map_err(js_err)?;
            }
            self.resizing = None;
            self.dragging = dragging;
            Ok(new_sel.is_some())
        }

        /// Pointer move with frame delta — drags or resizes the selection.
        pub fn on_move(&mut self, dx: f32, dy: f32) -> Result<(), JsValue> {
            let Some(id) = self.chart_selection() else {
                return Ok(());
            };
            let (dx, dy) = self.display_delta_to_document(dx, dy);
            let mut config = self.chart_config().clone();
            if let Some(handle) = self.resizing {
                if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable())
                    && rz.resize_by(&mut config, handle, dx, dy) == NudgeResult::Moved
                {
                    self.replace_chart_config(config)?;
                }
                return Ok(());
            }
            if self.dragging
                && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
                && drag.drag_by(&mut config, dx, dy) == NudgeResult::Moved
            {
                self.replace_chart_config(config)?;
            }
            Ok(())
        }

        pub fn on_release(&mut self) {
            self.dragging = false;
            self.resizing = None;
        }

        pub fn has_selection(&self) -> bool {
            self.chart_selection().is_some()
        }

        // ---- frame / resize / export ----

        /// Wait until previously submitted GPU work has finished. Does not
        /// draw. Used to separate an earlier compute submit from the next
        /// render submit when measuring startup.
        pub async fn wait_submitted_work(&self) {
            self.renderer.wait_submitted_work().await;
        }

        /// Compile the drawable-series fit-bound engine without drawing. On wasm
        /// this uses `createComputePipelineAsync`. Series registration does
        /// not do this; `auto_fit_all` does.
        pub async fn ensure_extent_engine(&mut self) -> Result<(), JsValue> {
            self.renderer
                .ensure_errorbar_extent_engine()
                .await
                .map_err(js_err)
        }

        /// Submit the current chart if it is dirty, then wait until that GPU
        /// work has finished. Hosts should keep a loading overlay up until
        /// this resolves. The wait does not remove a main-thread Dawn/driver
        /// freeze; it only reports first-submit completion.
        ///
        /// JS: `await chart.first_frame_ready();`
        pub async fn first_frame_ready(&mut self) -> Result<(), JsValue> {
            self.frame()?;
            self.renderer.wait_submitted_work().await;
            Ok(())
        }

        /// Alias for [`Self::first_frame_ready`].
        pub async fn warm_up(&mut self) -> Result<(), JsValue> {
            self.first_frame_ready().await
        }

        /// Compile every renderer pipeline while reporting the exact current
        /// stage as `{ scope, stage, phase }` objects. The chart is mutably
        /// borrowed until the Promise settles; hosts must keep their normal
        /// async busy gate active and queue chart work rather than calling the
        /// kernel from the callback.
        pub async fn prewarm_all_with_progress(
            &mut self,
            on_event: js_sys::Function,
        ) -> Result<(), JsValue> {
            let mut observer = |event| emit_init_progress(Some(&on_event), event);
            self.renderer
                .prewarm_all_observed(&mut observer)
                .await
                .map_err(js_err)?;
            self.request_host_redraw();
            Ok(())
        }

        /// Compile every renderer pipeline without a progress callback.
        pub async fn prewarm_all(&mut self) -> Result<(), JsValue> {
            self.renderer.prewarm_all().await.map_err(js_err)?;
            self.request_host_redraw();
            Ok(())
        }

        /// Process pending pool maintenance and draw only when visual state is
        /// dirty. A clean rAF tick returns before touching the pool or surface.
        pub fn frame(&mut self) -> Result<(), JsValue> {
            self.renderer
                .sync_external_invalidations()
                .map_err(js_err)?;
            let current_stamp = self
                .renderer
                .chart_render_stamp(self.chart_id)
                .map_err(js_err)?;
            let renderer_dirty = current_stamp.needs_draw_since(self.last_presented_stamp.as_ref());
            let raster_dirty = current_stamp.needs_raster_since(self.last_presented_stamp.as_ref());
            let decision = frame_decision(
                renderer_dirty,
                raster_dirty,
                self.view_dirty,
                self.redraw_pending,
                self.renderer.has_pending_maintenance(),
            );
            let refresh_raster = match decision {
                FrameDecision::Clean => return Ok(()),
                FrameDecision::MaintenanceOnly => {
                    let _ = self
                        .renderer
                        .process_pending_maintenance()
                        .map_err(js_err)?;
                    return Ok(());
                }
                FrameDecision::Draw { refresh_raster } => refresh_raster,
            };

            let _ = self
                .renderer
                .process_pending_maintenance()
                .map_err(js_err)?;
            let current_stamp = self
                .renderer
                .chart_render_stamp(self.chart_id)
                .map_err(js_err)?;

            // Browser resize is preview zoom, not a document mutation. The
            // stored chart remains the export SSoT; the live canvas renders a
            // scaled/letterboxed display chart derived from it.
            let (display_config, panel_rect, _) = self.display_config();
            let display_chart = Chart::new(display_config);
            if refresh_raster {
                let sel_boxes: Vec<SelectionBox> = self
                    .chart_selection()
                    .and_then(|id| {
                        self.hitmap.selection_box(
                            id,
                            display_chart.config(),
                            &CpuTextMeasure::for_style(&display_chart.config().draw_style),
                        )
                    })
                    .into_iter()
                    .collect();
                self.renderer
                    .refresh_axis_with_selection(
                        &mut self.view,
                        &display_chart,
                        panel_rect,
                        &sel_boxes,
                    )
                    .map_err(js_err)?;
            }

            if self.renderer.is_streaming_chart(self.chart_id) {
                self.renderer
                    .draw_registered(
                        self.clear_color,
                        &[RegisteredChartDrawItem {
                            chart_id: self.chart_id,
                            view: &self.view,
                        }],
                    )
                    .map_err(js_err)?;
            } else {
                let series_configs = self.chart_series().to_vec();
                let series: Vec<Series<'_>> = self
                    .styles
                    .iter()
                    .zip(series_configs.iter())
                    // A cancelled stream (or an unpublished residency upload)
                    // has no data surface. Its logical registration is not a
                    // resident allocation; keep drawing decorations and any
                    // independent resident series without requesting it from
                    // the column pool.
                    .filter(|(_, config)| {
                        !referenced_columns(config).into_iter().any(|id| matches!(
                            self.renderer.logical_column(id),
                            Some(renderer::LogicalColumn::Streamed(_))
                        ))
                    })
                    .map(|(style, config)| Series { config, style })
                    .collect();
                let items = [ChartDrawItem {
                    view: &self.view,
                    chart_config: display_chart.config(),
                    series: &series,
                }];
                self.renderer
                    .draw(self.clear_color, &items)
                    .map_err(js_err)?;
            }

            // Only a successfully submitted/presented draw advances the
            // renderer-issued onscreen stamp. Any earlier failure leaves the
            // frame dirty so the next persistent rAF retries it.
            consume_successful_frame(&mut self.view_dirty, &mut self.redraw_pending);
            self.last_presented_stamp = Some(current_stamp);
            Ok(())
        }

        /// Set the WebGPU surface clear color used behind the chart panel.
        /// Components are linear 0..1 RGBA floats. This is host/app state:
        /// it does not change the chart Config JSON.
        pub fn set_clear_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
            self.clear_color = Color::from_rgba(
                r.clamp(0.0, 1.0),
                g.clamp(0.0, 1.0),
                b.clamp(0.0, 1.0),
                a.clamp(0.0, 1.0),
            );
            self.request_host_redraw();
        }

        /// Resize the swap chain viewport. The chart Config keeps its
        /// document/export `chart_area`; live rendering scales that logical
        /// document into this surface.
        pub fn resize(&mut self, width: u32, height: u32) -> Result<(), JsValue> {
            let (w, h) = (width.max(1), height.max(1));
            self.renderer.resize(w, h).map_err(js_err)?;
            self.surface_size = (w, h);
            self.view_dirty = true;
            self.rebuild_styles();
            Ok(())
        }

        /// Export the panel as PNG bytes at `scale ×` resolution.
        /// JS: `const png = await chart.export_png(2.0);`
        /// (`&mut self`: the renderer's export runs its prepare phase —
        /// transform uniforms + arc-prefix compute; wasm-bindgen serializes
        /// access, so this changes nothing for JS callers.)
        /// Automatic contour placement validates `spacing_px * scale` after the
        /// export scale is clamped. Explicit placement does not multiply spacing.
        /// Scale-product overflow or an atlas beyond the adapter texture limit
        /// rejects the export before contour GPU state changes.
        pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
            let export_chart = Chart::new(self.chart_config().clone());
            let series = self.chart_series().to_vec();
            let bytes = self
                .renderer
                .export_panel_png_bytes_with_clear_async(
                    &export_chart,
                    &series,
                    scale,
                    self.clear_color,
                )
                .await
                .map_err(js_err)?;
            Ok(js_sys::Uint8Array::from(bytes.as_slice()))
        }

        /// Begin an independent replay of the completed stream. The facade
        /// supplies only renderer-requested ranges between these calls.
        pub fn begin_stream_export(
            &mut self, scale: f32, max_primitives_per_chunk: f64,
        ) -> Result<StreamingOperationHandle, JsValue> {
            let max_chunk = js_safe_u64(max_primitives_per_chunk, "max_primitives_per_chunk")?;
            let operation = self.renderer.begin_stream_export(
                self.chart_id, scale, self.clear_color, max_chunk,
            ).map_err(js_err)?;
            Ok(StreamingOperationHandle {
                operation,
                revision: self.renderer.stream_status(self.chart_id).map_err(js_err)?.revision.to_string(),
                counts: Cell::new((0.0, 0.0)),
            })
        }

        /// Compatibility entry point. Nonresident picking is disabled because
        /// exact source replay can take as long as the original render.
        pub async fn begin_stream_pick_point(
            &mut self, _x: f32, _y: f32, _max_distance_px: f32, _max_primitives_per_chunk: f64,
        ) -> Result<StreamingOperationHandle, JsValue> {
            Err(JsValue::from_str("stream picking is disabled; no source replay is started"))
        }

        /// Compatibility entry point; always rejects without requesting ranges.
        pub async fn begin_stream_pick_data(
            &mut self, _x: f32, _y: f32, _max_distance_px: f32, _max_primitives_per_chunk: f64,
        ) -> Result<StreamingOperationHandle, JsValue> {
            Err(JsValue::from_str("stream picking is disabled; no source replay is started"))
        }

        pub fn stream_operation_request_ranges(
            &mut self, handle: &StreamingOperationHandle,
        ) -> Result<AutoStreamingRangeRequestResult, JsValue> {
            let request = self.renderer.request_stream_operation_ranges(handle.operation).map_err(js_err)?;
            let response = AutoStreamingRangeRequestResult::from_core(
                request, handle.revision.clone(), handle.counts.get(),
            )?;
            handle.counts.set((response.submitted_primitives, response.total_primitives));
            Ok(response)
        }

        pub fn set_stream_operation_chunk_budget(
            &mut self, handle: &StreamingOperationHandle, max_primitives: f64,
        ) -> Result<(), JsValue> {
            let budget = js_safe_u64(max_primitives, "stream operation chunk budget")?;
            self.renderer.set_stream_operation_chunk_budget(handle.operation, budget).map_err(js_err)
        }

        pub fn stream_operation_submit_ranges(
            &mut self, handle: &StreamingOperationHandle, ids: Vec<String>,
            revisions: Vec<f64>, source_lengths: Vec<f64>, offsets: Vec<f64>,
            sources: js_sys::Array,
        ) -> Result<StreamingStepResult, JsValue> {
            let sources = js_stream_sources(&sources)?;
            let bindings = stream_range_bindings(&ids, &revisions, &source_lengths, &offsets, &sources)?;
            let progress = self.renderer.submit_stream_operation_ranges(handle.operation, &bindings).map_err(js_err)?;
            let (status, submitted, total) = match progress {
                renderer::StreamingProgress::Submitted { submitted_primitives, total_primitives } =>
                    ("submitted", submitted_primitives, total_primitives),
                renderer::StreamingProgress::Backpressure { submitted_primitives, total_primitives } =>
                    ("backpressure", submitted_primitives, total_primitives),
                renderer::StreamingProgress::AllSubmitted { total_primitives } =>
                    ("all_submitted", total_primitives, total_primitives),
            };
            handle.counts.set((submitted as f64, total as f64));
            Ok(StreamingStepResult {
                status, revision: handle.revision.clone(), submitted_primitives: submitted as f64,
                total_primitives: total as f64, pending_latest: false,
            })
        }

        pub async fn finish_stream_export(
            &mut self, handle: &StreamingOperationHandle,
        ) -> Result<js_sys::Uint8Array, JsValue> {
            let image = self.renderer.finish_stream_export(handle.operation).await.map_err(js_err)?;
            let bytes = renderer::encode_png(&image).map_err(js_err)?;
            Ok(js_sys::Uint8Array::from(bytes.as_slice()))
        }

        /// Compatibility entry point; no streamed pick operation can start.
        pub async fn finish_stream_pick_point(
            &mut self, _handle: &StreamingOperationHandle,
        ) -> Result<JsValue, JsValue> {
            Err(JsValue::from_str("stream picking is disabled"))
        }

        /// Compatibility entry point; no streamed pick operation can start.
        pub async fn finish_stream_pick_data(
            &mut self, _handle: &StreamingOperationHandle,
        ) -> Result<JsValue, JsValue> {
            Err(JsValue::from_str("stream picking is disabled"))
        }

        pub async fn cancel_stream_operation_and_wait(
            &mut self, handle: &StreamingOperationHandle,
        ) -> Result<(), JsValue> {
            self.renderer.cancel_stream_operation_and_wait(handle.operation).await.map_err(js_err)
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
                styles.push(self.renderer.create_style_for_series_scaled(item, scale));
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

    #[cfg(test)]
    mod tests {
        use std::{
            collections::HashMap,
            hash::{Hash, Hasher},
        };

        use wasm_bindgen::JsCast;
        use wasm_bindgen_test::*;

        use super::*;
        use crate::scalar_job::SeriesExtentStatus;

        wasm_bindgen_test_configure!(run_in_browser);

        #[derive(Debug, PartialEq)]
        struct PoolAuthority {
            buffer: u64,
            capacity: u64,
            generation: u32,
            used_bytes: u64,
            free_bytes: u64,
            slots: Vec<(String, u64, u64, usize, u32, u64, u64, Option<u64>)>,
        }

        #[derive(Debug, PartialEq)]
        struct FiggyAuthority {
            config: renderer::Config,
            series: Vec<SeriesConfig>,
            columns: HashMap<String, usize>,
            column_revisions: HashMap<String, u64>,
            next_column_revision: u64,
            styles: (usize, usize, usize),
            labels: Vec<Option<String>>,
            legend_auto_managed: bool,
            color_seq: usize,
            series_extents: HashMap<SeriesExtentKey, (usize, SeriesExtentStatus)>,
            chart_stamp: ChartRenderStamp,
            visual_revision: renderer::RenderRevision,
            picker_visible_selection: Option<HitId>,
            pending_maintenance: bool,
            pool: PoolAuthority,
        }

        fn canvas() -> HtmlCanvasElement {
            let document = web_sys::window()
                .expect("window")
                .document()
                .expect("document");
            let canvas = document
                .create_element("canvas")
                .expect("canvas element")
                .dyn_into::<HtmlCanvasElement>()
                .expect("HtmlCanvasElement");
            canvas.set_width(800);
            canvas.set_height(500);
            canvas
        }

        fn buffer_identity(buffer: &wgpu::Buffer) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            buffer.hash(&mut hasher);
            hasher.finish()
        }

        fn snapshot(chart: &FiggyChart) -> FiggyAuthority {
            let pool = chart.renderer.pool();
            let mut ids = chart.columns.keys().cloned().collect::<Vec<_>>();
            ids.sort();
            ids.dedup();
            let slots = ids
                .into_iter()
                .map(|id| {
                    let slot = pool
                        .slot(&id)
                        .expect("host registry and renderer pool stay synchronized");
                    (
                        id,
                        slot.offset,
                        slot.byte_size,
                        slot.len_values,
                        slot.generation,
                        slot.min.to_bits(),
                        slot.max.to_bits(),
                        slot.min_positive.map(f64::to_bits),
                    )
                })
                .collect();
            let series_extents = chart
                .series_extents
                .iter()
                .map(|(key, job)| (key.clone(), (Rc::as_ptr(job) as usize, job.status())))
                .collect();

            FiggyAuthority {
                config: chart.chart_config().clone(),
                series: chart.chart_series().to_vec(),
                columns: chart.columns.clone(),
                column_revisions: chart.column_revisions.clone(),
                next_column_revision: chart.next_column_revision,
                styles: (
                    chart.styles.as_ptr() as usize,
                    chart.styles.len(),
                    chart.styles.capacity(),
                ),
                labels: chart.labels.clone(),
                legend_auto_managed: chart.legend_auto_managed,
                color_seq: chart.color_seq,
                series_extents,
                chart_stamp: chart.renderer.chart_render_stamp(chart.chart_id).unwrap(),
                visual_revision: chart.renderer.visual_revision(),
                picker_visible_selection: chart.renderer.chart_selection(chart.chart_id).unwrap(),
                pending_maintenance: chart.renderer.has_pending_maintenance(),
                pool: PoolAuthority {
                    buffer: buffer_identity(pool.buffer()),
                    capacity: pool.capacity(),
                    generation: pool.generation(),
                    used_bytes: pool.used_bytes(),
                    free_bytes: pool.free_bytes(),
                    slots,
                },
            }
        }

        async fn chart_with_stable_extent() -> FiggyChart {
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            chart
                .register_column_f64("stable-x", &[0.0, 1.0, 2.0])
                .unwrap();
            chart
                .register_column_f64("stable-y", &[2.0, 3.0, 4.0])
                .unwrap();
            chart
                .add_line_series("stable", "stable-x", "stable-y", 2.0, "stable")
                .unwrap();
            chart.ensure_extent_engine().await.unwrap();
            chart.auto_fit_all(0.0).await.unwrap();
            chart
        }

        #[wasm_bindgen_test(async)]
        async fn real_load_demo_late_abort_preserves_every_authority_then_retries() {
            set_load_demo_abort_before_renderer_commit(false);
            reset_series_extent_submit_count();
            let mut chart = chart_with_stable_extent().await;
            chart.renderer.enable_gpu_picking_async().await.unwrap();
            chart
                .renderer
                .prepare_gpu_picking_for_chart(chart.chart_id)
                .unwrap();
            let before = snapshot(&chart);

            set_load_demo_abort_before_renderer_commit(true);
            let error = chart
                .load_demo()
                .expect_err("test hook must abort load_demo");
            set_load_demo_abort_before_renderer_commit(false);
            assert_eq!(
                error.as_string().as_deref(),
                Some("injected load_demo abort before renderer commit")
            );
            assert_eq!(snapshot(&chart), before);

            chart.load_demo().expect("retry must commit");
            for id in crate::DEMO_COLUMN_IDS {
                assert_eq!(chart.columns.get(id), Some(&512));
                assert!(chart.renderer.pool().slot(id).is_some());
            }
            assert!(
                chart
                    .chart_series()
                    .iter()
                    .any(|series| series.series_id == "sine")
            );
            assert!(
                chart
                    .chart_series()
                    .iter()
                    .any(|series| series.series_id == "rc")
            );
        }

        /// A matrix-shaped batch registers every column in one upload, and the
        /// f64 form keeps the `(hi, lo)` precision the single-column path does.
        ///
        /// `ColumnPool` creations answers the "one upload" claim: the batch takes a
        /// single staging buffer, so a 16-column grid must not create 16.
        #[wasm_bindgen_test(async)]
        async fn a_column_batch_registers_every_column_in_one_upload() {
            const N: usize = 16;
            let mut chart = FiggyChart::create(canvas()).await.expect("create chart");
            let ids: Vec<String> = (0..N).map(|c| format!("g{c}")).collect();
            // A grid whose values need the low half: the integer part alone would
            // round away the +0.125 / +0.875 offsets in f32.
            let data: Vec<f64> = (0..N)
                .flat_map(|c| (0..N).map(move |r| 1_700_000_000_000.0 + c as f64 + r as f64 / 8.0))
                .collect();

            let before = chart
                .renderer
                .gpu_memory_usage()
                .creations_of(renderer::GpuResourceKind::ColumnPool);
            chart
                .register_columns_f64(ids.clone(), &data, N)
                .expect("batch register");
            let creations = chart
                .renderer
                .gpu_memory_usage()
                .creations_of(renderer::GpuResourceKind::ColumnPool)
                - before;
            assert!(
                creations <= 1,
                "{N} columns must upload through one staging buffer, not {creations}"
            );

            for id in &ids {
                assert_eq!(chart.columns.get(id), Some(&N));
                assert!(chart.column_revisions.contains_key(id));
            }
            // One revision per column, not one for the batch.
            let mut revisions: Vec<u64> = ids
                .iter()
                .map(|id| *chart.column_revisions.get(id).unwrap())
                .collect();
            revisions.sort_unstable();
            revisions.dedup();
            assert_eq!(revisions.len(), N);

            // A second batch naming one of the same ids is rejected whole.
            let clash = vec![format!("fresh"), ids[0].clone()];
            let small: Vec<f64> = vec![0.0; 2 * N];
            assert!(chart.register_columns_f64(clash, &small, N).is_err());
            assert!(!chart.columns.contains_key("fresh"));
        }

        #[wasm_bindgen_test(async)]
        async fn load_demo_defers_real_extent_submits_and_lazy_fit_retains_stable_job() {
            set_load_demo_abort_before_renderer_commit(false);
            reset_series_extent_submit_count();
            let mut chart = chart_with_stable_extent().await;
            assert_eq!(series_extent_submit_count(), 1);

            let stable_key = crate::series_extent_request_from(
                &chart.column_revisions,
                &chart.chart_series()[0],
            )
            .unwrap()
            .key;
            let stable_job = Rc::clone(chart.series_extents.get(&stable_key).unwrap());
            let before_load_demo = series_extent_submit_count();

            chart.load_demo().unwrap();
            assert_eq!(series_extent_submit_count(), before_load_demo);
            assert!(Rc::ptr_eq(
                chart.series_extents.get(&stable_key).unwrap(),
                &stable_job
            ));

            let demo_keys = chart
                .chart_series()
                .iter()
                .filter(|series| series.series_id == "sine" || series.series_id == "rc")
                .map(|series| {
                    crate::series_extent_request_from(&chart.column_revisions, series)
                        .unwrap()
                        .key
                })
                .collect::<Vec<_>>();
            assert_eq!(demo_keys.len(), 2);
            assert!(
                demo_keys
                    .iter()
                    .all(|key| !chart.series_extents.contains_key(key))
            );

            let before_lazy_fit = series_extent_submit_count();
            chart.auto_fit_all(0.0).await.unwrap();
            assert_eq!(series_extent_submit_count(), before_lazy_fit + 2);
            assert!(Rc::ptr_eq(
                chart.series_extents.get(&stable_key).unwrap(),
                &stable_job
            ));
            for key in demo_keys {
                assert_eq!(
                    chart.series_extents.get(&key).unwrap().status(),
                    SeriesExtentStatus::Succeeded
                );
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
