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

/// The JSON of [`renderer::text::RichText::from_tex`]: the rich text the
/// `rich_text_from_tex` binding returns, ready for any `RichText` field of
/// `set_config` or `set_series`. The error names the TeX byte range.
#[cfg(any(target_arch = "wasm32", test))]
fn rich_text_json_from_tex(
    tex: &str,
    font_size: f32,
    font: &str,
    color: [f32; 4],
) -> Result<String, String> {
    let [r, g, b, a] = color;
    let text =
        renderer::text::RichText::from_tex(tex, renderer::Color::new(r, g, b, a), font_size, font)
            .map_err(|e| e.to_string())?;
    serde_json::to_string(&text).map_err(|e| e.to_string())
}

/// The JSON of [`renderer::layout::auto_layout`] (#44) on the config JSON
/// `json`, measured with the renderer's text measure for the config's draw
/// style: what the `autoLayout` binding returns, for `set_config`. Reads
/// only the JSON and the measure. The error is the JSON parse or write
/// error.
#[cfg(any(target_arch = "wasm32", test))]
fn auto_layout_json(json: &str) -> Result<String, String> {
    let config: renderer::Config = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    serde_json::to_string(&renderer::layout::auto_layout(&config, &measure))
        .map_err(|e| e.to_string())
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

/// A value label hit as `pick_point` reports it: the point fields with the
/// label's category as `point_index`, and the label's id, which
/// `press_value_label` takes.
#[cfg(any(target_arch = "wasm32", test))]
fn picked_label_json_string(
    source_id: Option<&str>,
    series_id: &str,
    category_index: usize,
    distance_px: f32,
) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": source_id,
        "series_id": series_id,
        "point_index": category_index,
        "distance_px": distance_px,
        "label_id": renderer::data_config::value_label_id(series_id, category_index),
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
        renderer::PickedData::ValueLabel {
            source_id,
            series_id,
            category_index,
            distance_px,
        } => serde_json::json!({
            "kind": "value_label",
            "source_id": source_id.as_ref(),
            "series_id": series_id,
            "category_index": category_index,
            "label_id": renderer::data_config::value_label_id(series_id, *category_index),
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
    /// A `Data`-scaled point vector's tips are `anchor + factor * (u, v)`,
    /// so the factor is part of the extent's identity; bits, so the key
    /// stays `Eq`.
    vector_factor_bits: Option<u64>,
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
        // Bars, fields, vectors and radar polygons carry no error columns.
        DataRenderType::Histogram { .. }
        | DataRenderType::CategoryBars { .. }
        | DataRenderType::BoxPlot { .. }
        | DataRenderType::Radar { .. }
        | DataRenderType::Heatmap { .. }
        | DataRenderType::Contour { .. }
        | DataRenderType::HeatmapContour { .. }
        | DataRenderType::Vector { .. } => (None, None),
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
        // A grid vector fits its `u` grid's lattice; a Data-scaled point
        // vector's tips depend on its u / v columns, which therefore join the
        // identity in the matrix slot.
        renderer::DataRenderType::Vector {
            components: renderer::data_config::VectorComponents::Grid { u, .. },
            ..
        } => (
            u.columns
                .iter()
                .map(|id| revisioned_column_from(revisions, id))
                .collect::<Option<Vec<_>>>()?,
            Some(match &u.orientation {
                renderer::data_config::MatrixOrientation::ColumnsAreX => 0,
                renderer::data_config::MatrixOrientation::ColumnsAreY => 1,
            }),
        ),
        renderer::DataRenderType::Vector {
            components: renderer::data_config::VectorComponents::Points { u, v },
            style,
        } => (
            match style.scale {
                renderer::data_config::VectorScale::Data { .. } => [u, v]
                    .into_iter()
                    .map(|id| revisioned_column_from(revisions, id))
                    .collect::<Option<Vec<_>>>()?,
                renderer::data_config::VectorScale::Pixels { .. } => Vec::new(),
            },
            None,
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
        | renderer::DataRenderType::Histogram { .. }
        | renderer::DataRenderType::CategoryBars { .. }
        | renderer::DataRenderType::BoxPlot { .. }
        | renderer::DataRenderType::Radar { .. } => (Vec::new(), None),
    };
    let vector_factor_bits = match &cfg.render_type {
        renderer::DataRenderType::Vector {
            components: renderer::data_config::VectorComponents::Points { .. },
            style,
        } => match style.scale {
            renderer::data_config::VectorScale::Data { factor } => Some(factor.to_bits()),
            renderer::data_config::VectorScale::Pixels { .. } => None,
        },
        _ => None,
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
            vector_factor_bits,
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

/// Replace the chart title with plain `text` and grow its band to hold it, as
/// `Chart::with_title` does natively, so a multi-line title set from the
/// browser is not clipped at the top of the chart.
#[cfg(any(target_arch = "wasm32", test))]
fn set_plain_chart_title(config: &mut renderer::Config, text: &str) {
    config.chart_title.text.segments = renderer::text::rich_segments_from_text(text);
    let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
    renderer::layout::fit_chart_title_margin(config, &measure);
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
                DataRenderType::CategoryBars { color, .. } => *color,
                DataRenderType::BoxPlot { color, .. } => *color,
                DataRenderType::Radar { color, .. } => *color,
                DataRenderType::Contour { contour, .. }
                | DataRenderType::HeatmapContour { contour, .. } => contour.line.line_color,
                // A heatmap's colour is the colourbar's colormap, not a swatch
                // this demo could reuse.
                DataRenderType::Heatmap { .. } => cycle.color(index),
                DataRenderType::Vector { style, .. } => style.color,
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
            align: config.legend.content.align,
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
                    fill: None,
                    fill_color: None,
                    interpolation: renderer::data_config::LineInterpolation::Linear,
                },
            },
        };
        if let Some(index) = existing {
            series[index] = item.clone();
            labels[index] = Some(label.to_string());
            renderer::config::set_legend_entry_label(
                &mut config.legend.content,
                index,
                renderer::config::series_symbol_segments(
                    &item,
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
                rich_label.segments,
            );
        } else {
            series.push(item.clone());
            labels.push(Some(label.to_string()));
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(
                    &item,
                    config.colorbar.as_ref().map(|bar| &bar.colormap),
                ),
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

#[cfg(all(test, not(target_arch = "wasm32")))]
mod series_extent_mutation_tests;

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
        frame_decision, picked_data_json_string, picked_label_json_string,
        picked_point_json_string, prepare_batch_column_metadata, prepare_column_metadata,
        prepare_demo_column_metadata, prepare_demo_declarations, retain_valid_series_extent_jobs,
        select_series_extent_job_map, series_extent_key_matches_config, series_extent_mode,
        series_extent_needs_submission, series_extent_request_from, set_plain_chart_title,
        validate_column_batch, validate_column_data_len, validate_column_registry_action,
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
            point_color_map: None,
            point_size_map: None,
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
            fill: None,
            fill_color: None,
            interpolation: renderer::data_config::LineInterpolation::Linear,
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

    // `set_title` grows the title band like `Chart::with_title`: a single
    // default-size line keeps the default band, and a three-line title gets a
    // band at least as tall as its block, so its first line is not clipped.
    #[test]
    fn set_plain_chart_title_grows_the_band_to_hold_a_multi_line_title() {
        use renderer::text_render::{FontPolicy, measure_rich_text};

        let default_band = renderer::default::default_config().chart_title.top_margin;
        let mut config = renderer::default::default_config();
        set_plain_chart_title(&mut config, "figgy");
        assert_eq!(config.chart_title.top_margin, default_band);

        set_plain_chart_title(&mut config, "first\nsecond\nthird");
        let block = measure_rich_text(&config.chart_title.text, FontPolicy::Standard);
        assert!(block.height() > default_band);
        assert!(config.chart_title.top_margin >= block.height());
        assert!(config.data_area().is_ok());
    }

    // `get_config` / `set_config` and `get_series` / `set_series` are
    // `serde_json` over `renderer::Config` and `SeriesConfig`: a segment's
    // `char_set` reaches JavaScript as `greek` plus `math` (written only for
    // `Math`), comes back from it unchanged, and a segment naming both sets
    // is rejected before `set_config` touches any state.
    #[test]
    fn config_and_series_json_carry_the_segment_char_set_both_ways() {
        use renderer::text::{CharSet, RichSegment};

        let mut config = renderer::default::default_config();
        config.chart_title.text.segments = vec![
            RichSegment {
                char_set: CharSet::Greek,
                ..RichSegment::plain('s')
            },
            RichSegment {
                char_set: CharSet::Math,
                ..RichSegment::plain('≤')
            },
            RichSegment::plain('1'),
        ];
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains(r#""text":"s","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":true}"#), "{json}");
        assert!(json.contains(r#""text":"≤","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false,"math":true}"#), "{json}");
        assert!(json.contains(r#""text":"1","bold":false,"italic":false,"underline":false,"superscript":false,"subscript":false,"greek":false}"#), "{json}");
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let both = json.replacen(r#""greek":true}"#, r#""greek":true,"math":true}"#, 1);
        assert_ne!(both, json);
        let err = serde_json::from_str::<renderer::Config>(&both)
            .expect_err("a segment cannot be greek and math at once")
            .to_string();
        assert!(err.contains("both"), "{err}");

        let mut series = vec![series_config(
            "s",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        )];
        series[0].label = Some(config.chart_title.text.clone());
        let json = serde_json::to_string(&series).unwrap();
        assert!(json.contains(r#""greek":false,"math":true}"#), "{json}");
        let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, series);
    }

    // The math fields reach JavaScript and come back unchanged through
    // `get_config`/`set_config` and `get_series`/`set_series`; a malformed
    // tree fails at parse, so `set_config` rejects it before it touches
    // any state; `rich_text_from_tex` writes the JSON a host puts there.
    // `autoLayout` is `layout::auto_layout` through JSON: the same document
    // as the Rust call, laid out again it is unchanged, and JSON that is not
    // a config is rejected without a result.
    #[test]
    fn auto_layout_json_is_the_rust_auto_layout() {
        let mut config = renderer::default::default_config();
        config.left_y.min = 0.0;
        config.left_y.max = 0.003;
        config.left_y.major_spacing = 0.0005;
        config.left_y.title_option.text =
            renderer::text::RichText::plain("Optical depth", renderer::Color::BLACK, 22.0, "");
        let json = serde_json::to_string(&config).unwrap();
        let out = super::auto_layout_json(&json).unwrap();
        let measure = renderer::CpuTextMeasure::for_style(&config.draw_style);
        let expected = renderer::layout::auto_layout(&config, &measure);
        assert_eq!(out, serde_json::to_string(&expected).unwrap());
        assert_ne!(expected, config, "the 0.00300 labels need a layout");
        assert_eq!(super::auto_layout_json(&out).unwrap(), out);
        assert!(super::auto_layout_json("{\"chart_area\": 1}").is_err());
        assert!(super::auto_layout_json("not json").is_err());
    }

    #[test]
    fn config_and_series_json_carry_math_nodes_both_ways() {
        let title = super::rich_text_json_from_tex(
            r"$\frac{\partial u}{\partial t} = \alpha \nabla^2 u$",
            18.0,
            "",
            [0.0, 0.0, 0.0, 1.0],
        )
        .unwrap();
        let title: renderer::text::RichText = serde_json::from_str(&title).unwrap();
        assert_eq!(title.validate_math(), Ok(()));
        let mut config = renderer::default::default_config();
        config.chart_title.text = title.clone();
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains(
                r#""math_node":{"id":1,"kind":{"Frac":"Auto"}},"math_parent":{"id":0,"slot":0}"#
            ),
            "{json}"
        );
        let back: renderer::Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back, config);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let broken = json.replacen(
            r#""math_parent":{"id":0,"slot":0}"#,
            r#""math_parent":{"id":7,"slot":0}"#,
            1,
        );
        assert_ne!(broken, json);
        let err = serde_json::from_str::<renderer::Config>(&broken)
            .expect_err("a malformed tree is refused at parse")
            .to_string();
        assert!(err.contains("parent"), "{err}");

        let mut series = vec![series_config(
            "s",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        )];
        series[0].label = Some(title);
        let json = serde_json::to_string(&series).unwrap();
        let back: Vec<SeriesConfig> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, series);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let err = super::rich_text_json_from_tex(r"$\frac{a}$", 12.0, "", [0.0; 4]).unwrap_err();
        assert!(
            err.contains("missing argument") && err.contains("1..6"),
            "{err}"
        );
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
    fn a_value_label_hit_names_its_category_and_its_label_id() {
        let picked = renderer::PickedData::ValueLabel {
            source_id: None,
            series_id: "bars:a".into(),
            category_index: 2,
            distance_px: 0.0,
        };
        let json: serde_json::Value =
            serde_json::from_str(&picked_data_json_string(&picked).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "value_label",
                "source_id": null,
                "series_id": "bars:a",
                "category_index": 2,
                "label_id": "series:bars:a:label:2",
                "distance_px": 0.0,
            })
        );
        let point: serde_json::Value =
            serde_json::from_str(&picked_label_json_string(None, "bars:a", 2, 0.0).unwrap())
                .unwrap();
        assert_eq!(point["point_index"], 2);
        assert_eq!(point["label_id"], "series:bars:a:label:2");
        assert_eq!(
            renderer::data_config::parse_value_label_id(point["label_id"].as_str().unwrap()),
            Some(("bars:a", 2))
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
        HiLoColumnSource, HitId, HitMap, LayerHit, RegisteredChartDrawItem, Renderer,
        ResizeHandle as ModelResizeHandle, Series, SeriesConfig, StreamColumn, StreamEncoding,
        StreamRangeSourceBinding, StreamReplay, StreamSourceBinding, StreamStatistics,
        StreamingChartOptions, StreamingLimits, WindowedRenderer,
    };

    use renderer::data_render::{AllocError, GrowthPolicy};
    use renderer::gpu_memory::GpuResourceKind;
    use renderer::{InitEvent, InitPhase};

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
        series_extent_key_matches_config, series_extent_mode, set_plain_chart_title,
        validate_column_batch, validate_column_data_len, validate_column_registry_action,
    };

    mod chart;
    use chart::PreparedSeriesExtentCache;
    pub use chart::{
        FiggyChart, auto_layout, draw_style_modes, draw_style_param_specs, rich_text_from_tex,
    };
    mod columns;
    use columns::{VectorColumns, referenced_columns, vector_columns};
    mod convert;
    pub use convert::{ColorCycle, color_cycle_css};
    use convert::{column_registration_error, js_err, js_safe_u64};
    mod decorations;
    pub use decorations::AxisPreset;
    pub(crate) use decorations::{annotation_kind_name, annotation_warnings_json};
    mod export;
    mod picking;
    mod streaming;
    use streaming::PendingStreamHandoffMetadata;
    pub use streaming::{AutoStreamingRangeRequestResult, AutoStreamingRequestResult};
    pub use streaming::{StreamingOperationHandle, StreamingStepResult, StreamingUsageResult};

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

#[cfg(target_arch = "wasm32")]
mod radial;
