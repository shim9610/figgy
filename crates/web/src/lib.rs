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
mod gpu_pick_style;
#[cfg(any(target_arch = "wasm32", test))]
mod scalar_job;

#[cfg(any(target_arch = "wasm32", test))]
fn fit_display_panel(
    logical_size: (u32, u32),
    surface_size: (u32, u32),
) -> (f32, renderer::layout::Rect) {
    let doc_w = logical_size.0.max(1) as f32;
    let doc_h = logical_size.1.max(1) as f32;
    let surface_w = surface_size.0.max(1);
    let surface_h = surface_size.1.max(1);
    let scale = ((surface_w as f32) / doc_w).min((surface_h as f32) / doc_h);
    let panel_w = ((doc_w * scale).round().max(1.0) as u32).min(surface_w);
    let panel_h = ((doc_h * scale).round().max(1.0) as u32).min(surface_h);
    let x = (surface_w - panel_w) / 2;
    let y = (surface_h - panel_h) / 2;
    (
        scale,
        renderer::layout::Rect {
            x,
            y,
            width: panel_w,
            height: panel_h,
        },
    )
}

#[cfg(any(target_arch = "wasm32", test))]
fn display_config_for_surface(
    config: &renderer::Config,
    surface_size: (u32, u32),
) -> (renderer::Config, renderer::layout::Rect, f32) {
    let logical = config.chart_area.0;
    let (scale, panel_rect) = fit_display_panel((logical.width, logical.height), surface_size);
    let mut display_config = config.scaled(scale);
    display_config.chart_area = renderer::layout::ChartArea(panel_rect);
    (display_config, panel_rect, scale)
}

#[cfg(any(target_arch = "wasm32", test))]
fn gpu_pick_query_for_surface(
    config: &renderer::Config,
    surface_size: (u32, u32),
    canvas_position_px: [f32; 2],
    max_distance_px: f32,
) -> (renderer::gpu_pick::GpuPickQuery, f32) {
    let (display_config, _, display_scale) = display_config_for_surface(config, surface_size);
    let chart_rect = display_config.chart_area.0;
    let data_area_px = display_config.data_area().ok().map(|area| {
        let rect = area.0;
        [
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        ]
    });
    (
        renderer::gpu_pick::GpuPickQuery {
            transform: renderer::data_render::scatter_transform_from_config(&display_config),
            chart_rect_px: [
                chart_rect.x as f32,
                chart_rect.y as f32,
                chart_rect.width as f32,
                chart_rect.height as f32,
            ],
            data_area_px,
            canvas_position_px,
            max_distance_px,
        },
        display_scale,
    )
}

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
    needs_defrag: bool,
) -> FrameDecision {
    let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
    match (visual_pending, needs_defrag) {
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RevisionedColumn {
    id: String,
    revision: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
fn series_extent_mode(render_type: &renderer::DataRenderType) -> renderer::GpuSeriesExtentMode {
    renderer::GpuSeriesExtentMode::from_render_type(render_type)
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

/// Exact GPU extent identity for one drawable series domain.
///
/// Inactive error directions are `None`; the renderer's internal filler
/// binding is deliberately excluded from this data identity.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SeriesExtentKey {
    mode: renderer::GpuSeriesExtentMode,
    x: RevisionedColumn,
    y: RevisionedColumn,
    x_error: Option<ErrorDirectionKey>,
    y_error: Option<ErrorDirectionKey>,
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeriesExtentColumnIds {
    x: String,
    y: String,
    x_lower: Option<String>,
    x_upper: Option<String>,
    y_lower: Option<String>,
    y_upper: Option<String>,
}

#[cfg(any(target_arch = "wasm32", test))]
impl SeriesExtentColumnIds {
    fn borrowed(&self) -> renderer::GpuSeriesExtentColumnIds<'_> {
        renderer::GpuSeriesExtentColumnIds {
            x: &self.x,
            y: &self.y,
            x_lower: self.x_lower.as_deref(),
            x_upper: self.x_upper.as_deref(),
            y_lower: self.y_lower.as_deref(),
            y_upper: self.y_upper.as_deref(),
        }
    }
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeriesExtentRequest {
    key: SeriesExtentKey,
    columns: SeriesExtentColumnIds,
}

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

    let x_lower = x_error.as_ref().map(|error| error.lower.id.clone());
    let x_upper = x_error.as_ref().map(|error| error.upper.id.clone());
    let y_lower = y_error.as_ref().map(|error| error.lower.id.clone());
    let y_upper = y_error.as_ref().map(|error| error.upper.id.clone());

    Some(SeriesExtentRequest {
        key: SeriesExtentKey {
            mode: series_extent_mode(&cfg.render_type),
            x: revisioned_column_from(revisions, &cfg.x_column)?,
            y: revisioned_column_from(revisions, &cfg.y_column)?,
            x_error,
            y_error,
        },
        columns: SeriesExtentColumnIds {
            x: cfg.x_column.clone(),
            y: cfg.y_column.clone(),
            x_lower,
            x_upper,
            y_lower,
            y_upper,
        },
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
    fn column_matches(
        column: &RevisionedColumn,
        id: &str,
        revisions: &std::collections::HashMap<String, u64>,
    ) -> bool {
        column.id == id && revisions.get(id) == Some(&column.revision)
    }

    fn error_matches(
        key: Option<&ErrorDirectionKey>,
        errors: Option<&renderer::data_config::ErrorRef>,
        revisions: &std::collections::HashMap<String, u64>,
    ) -> bool {
        use renderer::data_config::ErrorRef;

        match (key, errors) {
            (None, None) => true,
            (Some(key), Some(ErrorRef::Symmetric { column })) => {
                key.kind == ErrorRefKind::Symmetric
                    && column_matches(&key.lower, column, revisions)
                    && column_matches(&key.upper, column, revisions)
            }
            (Some(key), Some(ErrorRef::Asymmetric { lower, upper })) => {
                key.kind == ErrorRefKind::Asymmetric
                    && column_matches(&key.lower, lower, revisions)
                    && column_matches(&key.upper, upper, revisions)
            }
            _ => false,
        }
    }

    if key.mode != series_extent_mode(&cfg.render_type)
        || !column_matches(&key.x, &cfg.x_column, revisions)
        || !column_matches(&key.y, &cfg.y_column, revisions)
    {
        return false;
    }
    let (err_x, err_y) = err_refs(&cfg.render_type);
    error_matches(key.x_error.as_ref(), err_x, revisions)
        && error_matches(key.y_error.as_ref(), err_y, revisions)
}

#[cfg(any(target_arch = "wasm32", test))]
fn select_series_extent_job_map(
    ordered_requests: Vec<SeriesExtentRequest>,
    existing: &std::collections::HashMap<
        SeriesExtentKey,
        std::rc::Rc<crate::scalar_job::SeriesExtentJob>,
    >,
    retry_failed: bool,
) -> Result<
    (
        std::collections::HashMap<SeriesExtentKey, std::rc::Rc<crate::scalar_job::SeriesExtentJob>>,
        Vec<SeriesExtentRequest>,
    ),
    std::collections::TryReserveError,
> {
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
const INTERNAL_ZERO_COLUMN_ID: &str = "__zero";

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
fn validate_public_column_id(id: &str) -> Result<(), &'static str> {
    if id == INTERNAL_ZERO_COLUMN_ID {
        Err("is reserved for internal errorbar rendering")
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn column_update_invalidates_fit(id: &str) -> bool {
    id != INTERNAL_ZERO_COLUMN_ID
}

#[cfg(any(target_arch = "wasm32", test))]
fn validate_column_data_len(len: usize) -> Result<(), &'static str> {
    if len == 0 {
        Err("data must not be empty")
    } else {
        Ok(())
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn required_internal_zero_column_len(
    columns: &std::collections::HashMap<String, usize>,
    series_cfgs: &[renderer::SeriesConfig],
) -> usize {
    use renderer::data_config::ErrorRef;

    let mut needed = 0usize;
    for cfg in series_cfgs {
        let errors = match err_refs(&cfg.render_type) {
            (Some(errors), None) | (None, Some(errors)) => errors,
            (None, None) | (Some(_), Some(_)) => continue,
        };
        let mut count = columns
            .get(&cfg.x_column)
            .copied()
            .unwrap_or(0)
            .min(columns.get(&cfg.y_column).copied().unwrap_or(0));
        match errors {
            ErrorRef::Symmetric { column } => {
                count = count.min(columns.get(column).copied().unwrap_or(0));
            }
            ErrorRef::Asymmetric { lower, upper } => {
                count = count
                    .min(columns.get(lower).copied().unwrap_or(0))
                    .min(columns.get(upper).copied().unwrap_or(0));
            }
        }
        needed = needed.max(count);
    }
    needed
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, rc::Rc};

    use renderer::data_config::ErrorRef;
    use renderer::{
        Color, DataErrorBarStyleConfig, DataLineStyleConfig, DataRenderType,
        DataScatterStyleConfig, GpuSeriesExtentMode, ScatterShape, SeriesConfig,
    };

    use super::{
        ColumnRegistryAction, ErrorRefKind, FrameDecision, INTERNAL_ZERO_COLUMN_ID,
        active_series_extent_requests, checked_column_revision, column_update_invalidates_fit,
        consume_successful_frame, display_config_for_surface, fit_display_panel, frame_decision,
        gpu_pick_query_for_surface, picked_point_json_string, prepare_column_metadata,
        required_internal_zero_column_len, select_series_extent_job_map,
        series_extent_key_matches_config, series_extent_mode, series_extent_needs_submission,
        series_extent_request_from, validate_column_data_len, validate_column_registry_action,
        validate_public_column_id,
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

    #[test]
    fn frame_decision_covers_every_dirty_source_without_surface_work_when_clean() {
        for bits in 0u8..32 {
            let renderer_dirty = bits & 1 != 0;
            let raster_dirty = bits & 2 != 0;
            let view_dirty = bits & 4 != 0;
            let redraw_pending = bits & 8 != 0;
            let needs_defrag = bits & 16 != 0;
            let visual_pending = renderer_dirty || raster_dirty || view_dirty || redraw_pending;
            let expected = if !visual_pending {
                if needs_defrag {
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
                    needs_defrag,
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
        let (scale, panel) = fit_display_panel((1000, 800), (2000, 1600));
        assert!((scale - 2.0).abs() < 1e-6);
        assert_eq!(
            (panel.x, panel.y, panel.width, panel.height),
            (0, 0, 2000, 1600)
        );
    }

    #[test]
    fn display_panel_letterboxes_aspect_ratio_changes() {
        let (scale, panel) = fit_display_panel((1000, 800), (1600, 800));
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

        let (display, panel, scale) = display_config_for_surface(&config, (500, 400));

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
    fn gpu_pick_query_preserves_surface_scale_and_physical_distance_contract() {
        let mut config = renderer::default::default_config();
        config.chart_area = renderer::layout::ChartArea(renderer::layout::Rect {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        });

        for (surface, expected_scale, expected_rect) in [
            ((500, 400), 0.5, [0.0, 0.0, 500.0, 400.0]),
            ((1000, 800), 1.0, [0.0, 0.0, 1000.0, 800.0]),
            ((2000, 1600), 2.0, [0.0, 0.0, 2000.0, 1600.0]),
            ((1600, 800), 1.0, [300.0, 0.0, 1000.0, 800.0]),
        ] {
            let (query, display_scale) =
                gpu_pick_query_for_surface(&config, surface, [17.0, 23.0], 7.5);
            assert!((display_scale - expected_scale).abs() < 1e-6);
            assert_eq!(query.chart_rect_px, expected_rect);
            assert_eq!(query.canvas_position_px, [17.0, 23.0]);
            assert_eq!(query.max_distance_px, 7.5);
        }
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
            assert_eq!(series_extent_mode(&render_type), expected);
        }
    }

    #[test]
    fn series_extent_key_is_role_explicit_and_excludes_inactive_fillers() {
        let revisions = HashMap::from([
            ("x".to_string(), 1),
            ("y".to_string(), 2),
            ("err".to_string(), 3),
            (INTERNAL_ZERO_COLUMN_ID.to_string(), 4),
        ]);
        let plain = series_config(
            "plain",
            DataRenderType::Scatter {
                scatter: scatter_style(),
            },
        );
        let plain_request = series_extent_request_from(&revisions, &plain).unwrap();
        assert_eq!(plain_request.key.mode, GpuSeriesExtentMode::Points);
        assert!(plain_request.key.x_error.is_none());
        assert!(plain_request.key.y_error.is_none());
        assert_eq!(plain_request.columns.x_lower, None);
        assert_eq!(plain_request.columns.x_upper, None);
        assert_eq!(plain_request.columns.y_lower, None);
        assert_eq!(plain_request.columns.y_upper, None);
        let borrowed = plain_request.columns.borrowed();
        assert_eq!(borrowed.x, "x");
        assert_eq!(borrowed.y, "y");
        assert_eq!(borrowed.x_lower, None);
        assert_eq!(borrowed.x_upper, None);
        assert_eq!(borrowed.y_lower, None);
        assert_eq!(borrowed.y_upper, None);
        assert!(!format!("{:?}", plain_request.key).contains(INTERNAL_ZERO_COLUMN_ID));
        let mut filler_changed = revisions.clone();
        filler_changed.insert(INTERNAL_ZERO_COLUMN_ID.to_string(), 99);
        assert_eq!(
            series_extent_request_from(&filler_changed, &plain)
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
        let symmetric_request = series_extent_request_from(&revisions, &symmetric).unwrap();
        let symmetric_columns = symmetric_request.columns.borrowed();
        assert_eq!(symmetric_columns.x_lower, Some("err"));
        assert_eq!(symmetric_columns.x_upper, Some("err"));
        assert_eq!(symmetric_columns.y_lower, None);
        assert_eq!(symmetric_columns.y_upper, None);
        let symmetric_key = symmetric_request.key;
        let asymmetric_key = series_extent_request_from(&revisions, &asymmetric)
            .unwrap()
            .key;
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
    fn internal_zero_column_id_is_not_publicly_mutable() {
        assert_eq!(
            validate_public_column_id(INTERNAL_ZERO_COLUMN_ID),
            Err("is reserved for internal errorbar rendering")
        );
        assert_eq!(validate_public_column_id("x"), Ok(()));
        assert!(!column_update_invalidates_fit(INTERNAL_ZERO_COLUMN_ID));
        assert!(column_update_invalidates_fit("x"));
        assert_eq!(validate_column_data_len(0), Err("data must not be empty"));
        assert_eq!(validate_column_data_len(1), Ok(()));
    }

    #[test]
    fn internal_zero_length_matches_single_axis_draw_count_only() {
        let columns = HashMap::from([
            ("x".to_string(), 100),
            ("y".to_string(), 80),
            ("x_lo".to_string(), 60),
            ("x_hi".to_string(), 70),
            ("y_err".to_string(), 50),
        ]);
        let x_only = series_config(
            "x-only",
            DataRenderType::ScatterErrorbarX {
                scatter: scatter_style(),
                err_x: ErrorRef::Asymmetric {
                    lower: "x_lo".to_string(),
                    upper: "x_hi".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        let y_only = series_config(
            "y-only",
            DataRenderType::ScatterErrorbarY {
                scatter: scatter_style(),
                err_y: ErrorRef::Symmetric {
                    column: "y_err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );
        let xy = series_config(
            "xy",
            DataRenderType::ScatterErrorbarXY {
                scatter: scatter_style(),
                err_x: ErrorRef::Symmetric {
                    column: "x_lo".to_string(),
                },
                err_y: ErrorRef::Symmetric {
                    column: "y_err".to_string(),
                },
                err_style: errorbar_style(),
            },
        );

        assert_eq!(required_internal_zero_column_len(&columns, &[x_only]), 60);
        assert_eq!(required_internal_zero_column_len(&columns, &[y_only]), 50);
        assert_eq!(required_internal_zero_column_len(&columns, &[xy]), 0);
    }
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::{cell::Cell, collections::HashMap, rc::Rc, sync::Arc};

    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{future_to_promise, spawn_local};
    use web_sys::HtmlCanvasElement;

    use renderer::data_config::ErrorRef;
    use renderer::data_render::ColumnPool;
    use renderer::gpu_pick::{GpuPickEngine, GpuPickSeriesReplacement, PreparedGpuPickSeriesBatch};
    use renderer::layout::{ChartArea, NudgeResult, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::{RichText, rich_segments_from_text};
    use renderer::{
        Chart, ChartDrawItem, ChartId, ChartRenderStamp, ChartStyle, ChartView, Color,
        ColumnSource, CpuTextMeasure, DataLineStyleConfig, DataRenderType, DefragPolicy, FitExtent,
        HiLoColumnSource, HitId, HitMap, Renderer, ResizeHandle as ModelResizeHandle, SelectionBox,
        Series, SeriesConfig, WindowedRenderer,
    };

    use crate::borrowed_column::{BorrowedCastF32Column, BorrowedF32Column, BorrowedF64Column};
    use crate::gpu_pick_style::OwnedGpuPickSeriesDescriptor;
    use crate::scalar_job::{SeriesExtentJob, SeriesFitExtent};
    use crate::{
        ColumnRegistryAction, FrameDecision, INTERNAL_ZERO_COLUMN_ID, SeriesExtentKey,
        active_series_extent_requests, column_update_invalidates_fit, consume_successful_frame,
        frame_decision, prepare_column_metadata, required_internal_zero_column_len,
        select_series_extent_job_map, series_extent_key_matches_config, validate_column_data_len,
        validate_column_registry_action, validate_public_column_id,
    };

    const POOL_CAPACITY: u64 = 16 * 1024 * 1024;

    fn js_err(e: impl std::fmt::Display) -> JsValue {
        JsValue::from_str(&e.to_string())
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
        }
        ids
    }

    enum ColumnUploadSource<'a> {
        Scalar(&'a dyn ColumnSource),
        HiLo(&'a dyn HiLoColumnSource),
    }

    struct PickerReplacementPlan {
        gpu_index: usize,
        descriptor: OwnedGpuPickSeriesDescriptor,
    }

    enum PickerUploadPlan {
        Rebuild(Vec<OwnedGpuPickSeriesDescriptor>),
        Batch(Vec<PickerReplacementPlan>),
    }

    enum PreparedColumnPicker {
        Rebuild(GpuPickEngine),
        Batch(PreparedGpuPickSeriesBatch),
    }

    struct PreparedSeriesExtentCache {
        extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
        ticket_jobs: Vec<(renderer::GpuSeriesExtentTicket, Rc<SeriesExtentJob>)>,
    }

    struct PendingFitCommit {
        token: renderer::FitCommitToken,
        expected_epoch: u64,
        x_extent: FitExtent,
        y_extent: FitExtent,
        padding: f64,
        completion: futures_channel::oneshot::Sender<Result<(), String>>,
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

    #[wasm_bindgen]
    pub struct FiggyChart {
        renderer: WindowedRenderer<'static>,
        chart_id: ChartId,
        gpu_picker: GpuPickEngine,
        gpu_picker_dirty: bool,
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
        /// Already-submitted exact GPU drawable-domain reductions. Cache
        /// identity includes the normalized primitive mode and every active
        /// role-specific column revision.
        series_extents: HashMap<SeriesExtentKey, Rc<SeriesExtentJob>>,
        /// Invalidates an async fit if a later data/series/range mutation wins.
        fit_epoch: Rc<Cell<u64>>,
        alive: Rc<Cell<bool>>,
        /// Async readback completion mailbox. `frame()` is the only owner that
        /// mutates the renderer and commits a still-current fit token.
        pending_fit_commit: Rc<Cell<Option<PendingFitCommit>>>,
        /// A removal happened — defragment once on the next frame.
        needs_defrag: bool,
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
            super::fit_display_panel((logical.width, logical.height), self.surface_size)
        }

        fn display_config(&self) -> (renderer::config::Config, Rect, f32) {
            super::display_config_for_surface(self.chart_config(), self.surface_size)
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

        fn append_legend_entry_for(&mut self, i: usize) -> Result<(), JsValue> {
            let Some(cfg) = self.chart_series().get(i).cloned() else {
                return Ok(());
            };
            let Some(label) = self.fallback_label(i) else {
                return Ok(());
            };
            let mut config = self.chart_config().clone();
            renderer::config::append_legend_entry_rich(
                &mut config.legend.content,
                renderer::config::series_symbol_segments(&cfg),
                label.segments,
            );
            config.legend.visible = true;
            self.replace_chart_config(config)
        }

        fn remove_legend_entry_for(&mut self, i: usize) -> Result<(), JsValue> {
            let mut config = self.chart_config().clone();
            if renderer::config::remove_legend_entry(&mut config.legend.content, i)
                && config.legend.content.segments.is_empty()
            {
                config.legend.visible = false;
            }
            self.replace_chart_config(config)
        }

        fn sync_legend_symbols(&mut self, append_missing: bool) -> Result<(), JsValue> {
            let series = self.chart_series().to_vec();
            let mut config = self.chart_config().clone();
            let existing = renderer::config::legend_entry_count(&config.legend.content);
            renderer::config::update_legend_symbols_preserving_text(
                &mut config.legend.content,
                &series,
            );
            self.replace_chart_config(config)?;
            if append_missing {
                if existing > series.len() {
                    for i in (series.len()..existing).rev() {
                        self.remove_legend_entry_for(i)?;
                    }
                }
                for i in existing..series.len() {
                    self.append_legend_entry_for(i)?;
                }
            }
            Ok(())
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

        fn pick_series_active_in_metadata(&self, cfg: &SeriesConfig) -> bool {
            Self::pick_series_active_in(&self.columns, cfg)
        }

        fn pick_series_active_in(columns: &HashMap<String, usize>, cfg: &SeriesConfig) -> bool {
            columns
                .get(&cfg.x_column)
                .zip(columns.get(&cfg.y_column))
                .is_some_and(|(x_len, y_len)| (*x_len).min(*y_len) > 0)
        }

        fn pick_series_active_in_pool(&self, cfg: &SeriesConfig) -> bool {
            self.renderer
                .pool()
                .handle_for(&cfg.x_column)
                .zip(self.renderer.pool().handle_for(&cfg.y_column))
                .is_some_and(|(x, y)| x.len_values.min(y.len_values) > 0)
        }

        fn build_gpu_picker_for(
            &self,
            series_cfgs: &[SeriesConfig],
            precise: bool,
        ) -> Result<GpuPickEngine, JsValue> {
            let mut picker = GpuPickEngine::new(
                Arc::clone(self.renderer.device()),
                Arc::clone(self.renderer.queue()),
            )
            .map_err(js_err)?;
            for cfg in series_cfgs {
                if !self.pick_series_active_in_pool(cfg) {
                    continue;
                }
                let owned = OwnedGpuPickSeriesDescriptor::from_series(cfg, precise);
                owned
                    .with_descriptor(|descriptor| {
                        picker.add_series(self.renderer.pool(), descriptor)
                    })
                    .map_err(js_err)?;
            }
            Ok(picker)
        }

        fn build_gpu_picker_from_descriptors(
            device: Arc<wgpu::Device>,
            queue: Arc<wgpu::Queue>,
            pool: &ColumnPool,
            descriptors: &[OwnedGpuPickSeriesDescriptor],
        ) -> Result<GpuPickEngine, JsValue> {
            let mut picker = GpuPickEngine::new(device, queue).map_err(js_err)?;
            for owned in descriptors {
                owned.with_descriptor(|descriptor| {
                    let active = pool
                        .handle_for(&descriptor.x_column)
                        .zip(pool.handle_for(&descriptor.y_column))
                        .is_some_and(|(x, y)| x.len_values.min(y.len_values) > 0);
                    if active {
                        picker
                            .add_series(pool, descriptor)
                            .map(|_| ())
                            .map_err(js_err)
                    } else {
                        Ok(())
                    }
                })?;
            }
            Ok(picker)
        }

        fn prepare_picker_upload_plan(
            &self,
            id: &str,
            precise: bool,
        ) -> Result<PickerUploadPlan, JsValue> {
            if self.gpu_picker_dirty {
                let series = self.chart_series();
                let mut descriptors = Vec::new();
                descriptors.try_reserve(series.len()).map_err(js_err)?;
                descriptors.extend(
                    series
                        .iter()
                        .map(|cfg| OwnedGpuPickSeriesDescriptor::from_series(cfg, precise)),
                );
                return Ok(PickerUploadPlan::Rebuild(descriptors));
            }

            let mut replacements = Vec::new();
            replacements
                .try_reserve(self.chart_series().len())
                .map_err(js_err)?;
            let mut gpu_index = 0usize;
            for cfg in self.chart_series() {
                if !self.pick_series_active_in_metadata(cfg) {
                    continue;
                }
                let descriptor = OwnedGpuPickSeriesDescriptor::from_series(cfg, precise);
                if descriptor.references_column(id) {
                    replacements.push(PickerReplacementPlan {
                        gpu_index,
                        descriptor,
                    });
                }
                gpu_index += 1;
            }
            Ok(PickerUploadPlan::Batch(replacements))
        }

        fn repair_gpu_picker_if_dirty(&mut self) -> Result<(), JsValue> {
            if self.gpu_picker_dirty {
                self.rebuild_gpu_picker_now()?;
            }
            Ok(())
        }

        fn rebuild_gpu_picker_now(&mut self) -> Result<(), JsValue> {
            self.gpu_picker.clear_series();
            let precise = self.chart_config().draw_style.is_precise();
            let picker = self.build_gpu_picker_for(self.chart_series(), precise)?;
            self.gpu_picker = picker;
            self.gpu_picker_dirty = false;
            Ok(())
        }

        /// Invalidate an in-flight fit when a later mutation wins call order.
        fn bump_fit_epoch(&self) {
            self.fit_epoch.set(self.fit_epoch.get().wrapping_add(1));
        }

        fn commit_pending_fit(&mut self) {
            let Some(pending) = self.pending_fit_commit.take() else {
                return;
            };
            let result = if self.fit_epoch.get() != pending.expected_epoch {
                Err(
                    "auto_fit_all was superseded by a later fit, data, series, or axis mutation"
                        .to_string(),
                )
            } else {
                self.renderer
                    .commit_auto_fit_all_if_current(
                        &pending.token,
                        &pending.x_extent,
                        &pending.y_extent,
                        pending.padding,
                    )
                    .map_err(|error| error.to_string())
            };
            let _ = pending.completion.send(result);
        }

        fn request_host_redraw(&mut self) {
            self.redraw_pending = true;
        }
        fn prepare_series_extent_cache(
            &self,
            series_cfgs: &[SeriesConfig],
            revisions: &HashMap<String, u64>,
            retry_failed: bool,
        ) -> Result<PreparedSeriesExtentCache, JsValue> {
            let requests = active_series_extent_requests(series_cfgs, revisions).map_err(js_err)?;
            let (extents, pending) =
                select_series_extent_job_map(requests, &self.series_extents, retry_failed)
                    .map_err(js_err)?;
            let mut ticket_jobs = Vec::new();
            ticket_jobs.try_reserve(pending.len()).map_err(js_err)?;

            for request in pending {
                let job = extents.get(&request.key).cloned().ok_or_else(|| {
                    js_err("pending series extent key lost its prepared job before publication")
                })?;
                match self
                    .renderer
                    .begin_series_extent(request.key.mode, request.columns.borrowed())
                {
                    Ok(ticket) => ticket_jobs.push((ticket, job)),
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
            let series_cfgs = self.chart_series().to_vec();
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
            let precise = self.chart_config().draw_style.is_precise();
            let picker_plan = self.prepare_picker_upload_plan(id, precise)?;
            let device = Arc::clone(self.renderer.device());
            let queue = Arc::clone(self.renderer.queue());
            let mut ticket_jobs = Vec::new();
            ticket_jobs
                .try_reserve(pending_series_extents.len())
                .map_err(js_err)?;

            let guard = match source {
                ColumnUploadSource::Scalar(source) => self.renderer.begin_upsert_column(id, source),
                ColumnUploadSource::HiLo(source) => {
                    self.renderer.begin_upsert_hilo_column(id, source)
                }
            }
            .map_err(js_err)?;
            let replaced_existing = guard.replaced_existing();

            let prepared_picker = match &picker_plan {
                PickerUploadPlan::Rebuild(descriptors) => {
                    PreparedColumnPicker::Rebuild(Self::build_gpu_picker_from_descriptors(
                        device,
                        queue,
                        guard.pool(),
                        descriptors,
                    )?)
                }
                PickerUploadPlan::Batch(replacements) => {
                    let batch = self
                        .gpu_picker
                        .prepare_series_batch(
                            guard.pool(),
                            replacements
                                .iter()
                                .map(|replacement| GpuPickSeriesReplacement {
                                    gpu_index: replacement.gpu_index,
                                    descriptor: replacement.descriptor.descriptor(),
                                }),
                        )
                        .map_err(js_err)?;
                    PreparedColumnPicker::Batch(batch)
                }
            };

            for request in &pending_series_extents {
                let ticket = guard
                    .begin_series_extent(request.key.mode, request.columns.borrowed())
                    .map_err(js_err)?;
                let job = future_series_extents
                    .get(&request.key)
                    .cloned()
                    .ok_or_else(|| {
                        js_err("pending series extent key lost its prepared job before commit")
                    })?;
                ticket_jobs.push((ticket, job));
            }
            let prepared_series_extents = PreparedSeriesExtentCache {
                extents: future_series_extents,
                ticket_jobs,
            };

            guard.commit();
            match prepared_picker {
                PreparedColumnPicker::Rebuild(picker) => self.gpu_picker = picker,
                PreparedColumnPicker::Batch(batch) => self.gpu_picker.commit_series_batch(batch),
            }
            self.gpu_picker_dirty = false;
            self.columns = metadata.columns;
            self.column_revisions = metadata.revisions;
            self.next_column_revision = metadata.next_revision;
            self.needs_defrag |= replaced_existing;
            if column_update_invalidates_fit(id) {
                self.bump_fit_epoch();
            }
            self.publish_series_extent_cache(prepared_series_extents);
            Ok(())
        }

        fn upsert_zero_column(&mut self, len: usize) -> Result<(), JsValue> {
            if self.columns.get(INTERNAL_ZERO_COLUMN_ID) == Some(&len) {
                return Ok(());
            }
            let metadata = prepare_column_metadata(
                &self.columns,
                &self.column_revisions,
                self.next_column_revision,
                INTERNAL_ZERO_COLUMN_ID,
                len,
            )
            .map_err(js_err)?;
            let replaced_existing = self.columns.contains_key(INTERNAL_ZERO_COLUMN_ID);
            self.renderer
                .ensure_internal_zero_column(len)
                .map_err(js_err)?;
            self.columns = metadata.columns;
            self.column_revisions = metadata.revisions;
            self.next_column_revision = metadata.next_revision;
            self.needs_defrag |= replaced_existing;
            Ok(())
        }

        fn upsert_column_f64_as_f32(&mut self, id: &str, data: &[f64]) -> Result<(), JsValue> {
            let column = BorrowedCastF32Column::new(data);
            self.upsert_column_atomic(id, data.len(), ColumnUploadSource::Scalar(&column))
        }

        fn ensure_columns_exist(&self, cfg: &SeriesConfig) -> Result<(), JsValue> {
            for id in referenced_columns(cfg) {
                validate_public_column_id(id)
                    .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
                if !self.columns.contains_key(id) {
                    return Err(js_err(format!(
                        "series '{}' references unregistered column '{id}'",
                        cfg.series_id
                    )));
                }
            }
            Ok(())
        }

        /// Errorbar variants bind the internal `"__zero"` column for their
        /// unused error dimension. Rendering preparation owns this resource;
        /// changing the active series never uploads column data.
        fn ensure_zero_column_for_render(&mut self) -> Result<(), JsValue> {
            let needed = required_internal_zero_column_len(&self.columns, self.chart_series());
            let existing = self
                .columns
                .get(INTERNAL_ZERO_COLUMN_ID)
                .copied()
                .unwrap_or(0);
            if needed > 0 && existing < needed {
                self.upsert_zero_column(needed)?;
            }
            Ok(())
        }

        fn process_pending_defrag(&mut self) -> Result<bool, JsValue> {
            if !self.needs_defrag {
                return Ok(false);
            }
            // Preserve the existing R2-04 behavior: maintenance is consumed
            // before defrag/rebind. Visual dirty state is handled separately
            // and is never consumed here.
            self.needs_defrag = false;
            let relocated = self.renderer.defragment().map_err(js_err)?;
            if relocated {
                if !self.gpu_picker_dirty {
                    self.gpu_picker
                        .rebind_columns(self.renderer.pool())
                        .map_err(js_err)?;
                }
            }
            Ok(relocated)
        }
    }

    impl Drop for FiggyChart {
        fn drop(&mut self) {
            self.alive.set(false);
            self.bump_fit_epoch();
            if let Some(pending) = self.pending_fit_commit.take() {
                let _ = pending.completion.send(Err(
                    "chart was dropped while auto_fit_all was pending".into(),
                ));
            }
        }
    }

    #[wasm_bindgen]
    impl FiggyChart {
        /// Bind the low-level chart kernel to `canvas` (uses the canvas's
        /// current pixel size). Ordinary web hosts should prefer
        /// `figgy-chart.js` and its `<figgy-chart>` Custom Element.
        /// JS: `const chart = await FiggyChart.create(canvas);`
        pub async fn create(canvas: HtmlCanvasElement) -> Result<FiggyChart, JsValue> {
            console_error_panic_hook::set_once();

            let (w, h) = (canvas.width().max(1), canvas.height().max(1));
            let mut renderer = Renderer::for_window_async(
                wgpu::SurfaceTarget::Canvas(canvas),
                (w, h),
                POOL_CAPACITY,
            )
            .await
            .map_err(js_err)?;
            // Replace-heavy hosts can hit transient fragmentation between the
            // remove and the next frame's defrag — let the pool self-heal.
            renderer.set_defrag_policy(DefragPolicy::OnAllocFailure);
            let gpu_picker =
                GpuPickEngine::new(Arc::clone(renderer.device()), Arc::clone(renderer.queue()))
                    .map_err(js_err)?;

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

            Ok(FiggyChart {
                renderer,
                chart_id,
                gpu_picker,
                gpu_picker_dirty: false,
                view,
                surface_size: (w, h),
                styles: Vec::new(),
                labels: Vec::new(),
                legend_auto_managed: true,
                columns: HashMap::new(),
                column_revisions: HashMap::new(),
                next_column_revision: 1,
                series_extents: HashMap::new(),
                fit_epoch: Rc::new(Cell::new(0)),
                alive: Rc::new(Cell::new(true)),
                pending_fit_commit: Rc::new(Cell::new(None)),
                needs_defrag: false,
                color_seq: 0,
                hitmap: HitMap::standard_chart(),
                dragging: false,
                resizing: None,
                cycle: renderer::ColorCycle::Classic,
                clear_color: Color::WHITE,
                view_dirty: false,
                redraw_pending: true,
                last_presented_stamp: None,
            })
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

        // ---- column registry (explicit register / update / unregister) ----

        /// Register a new `Float32Array` column. Existing ids are rejected.
        pub fn register_column_f32(&mut self, id: &str, data: &[f32]) -> Result<(), JsValue> {
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
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
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
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

        /// Atomically replace an existing column from a `Float32Array`.
        /// Missing ids are rejected; every accepted call performs an upload.
        pub fn update_register_column_f32(
            &mut self,
            id: &str,
            data: &[f32],
        ) -> Result<(), JsValue> {
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
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
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
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
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
            if !self.columns.contains_key(id) {
                return Ok(false);
            }
            let old_series = self.chart_series().to_vec();
            if !self.renderer.remove_column(id).map_err(js_err)? {
                return Ok(false);
            }
            let was_dirty = self.gpu_picker_dirty;
            let keep: Vec<bool> = old_series
                .iter()
                .map(|cfg| !referenced_columns(cfg).contains(&id))
                .collect();
            let removed: Vec<usize> = keep
                .iter()
                .enumerate()
                .filter_map(|(i, keep)| (!keep).then_some(i))
                .collect();
            if !was_dirty {
                for &logical_index in removed.iter().rev() {
                    if self.pick_series_active_in_metadata(&old_series[logical_index]) {
                        let gpu_index = old_series[..logical_index]
                            .iter()
                            .filter(|cfg| self.pick_series_active_in_metadata(cfg))
                            .count();
                        if self.gpu_picker.remove_series_at(gpu_index).is_err() {
                            self.gpu_picker_dirty = true;
                            break;
                        }
                    }
                }
            }

            self.columns.remove(id);
            self.column_revisions.remove(id);
            debug_assert!(self.renderer.pool().slot(id).is_none());
            self.needs_defrag = true;

            if keep.iter().any(|k| !k) {
                let mut it = keep.iter();
                self.styles.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.labels.retain(|_| *it.next().unwrap());
                if self.legend_auto_managed {
                    for i in removed.into_iter().rev() {
                        self.remove_legend_entry_for(i)?;
                    }
                } else {
                    self.sync_legend_symbols(false)?;
                }
            }
            if self.gpu_picker_dirty {
                let _ = self.rebuild_gpu_picker_now();
            }
            self.bump_fit_epoch();
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
            let precise = self.chart_config().draw_style.is_precise();
            let next_gpu_picker = self.build_gpu_picker_for(&proposed, precise)?;
            let prepared_extents =
                self.prepare_series_extent_cache(&proposed, &self.column_revisions, false)?;
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
            self.gpu_picker = next_gpu_picker;
            self.gpu_picker_dirty = false;
            self.color_seq = next_color_seq;
            self.bump_fit_epoch();
            self.publish_series_extent_cache(prepared_extents);
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
            let precise = self.chart_config().draw_style.is_precise();
            let next_gpu_picker = self.build_gpu_picker_for(&series, precise)?;
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
            self.gpu_picker = next_gpu_picker;
            self.gpu_picker_dirty = false;
            self.bump_fit_epoch();
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
            self.bump_fit_epoch();
            Ok(())
        }

        pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            let mut chart = Chart::new(self.chart_config().clone());
            chart
                .auto_fit_y(self.renderer.pool(), column, padding)
                .map_err(js_err)?;
            self.replace_chart_config(chart.config().clone())?;
            self.bump_fit_epoch();
            Ok(())
        }

        /// Fit BOTH axes to the union of every registered series, leaving a
        /// uniform `padding` fraction of the data span as margin on each
        /// side (`0.0` = exact fit, `0.05` = 5% top/bottom/left/right).
        /// This is the whole fit policy — no rounding of the range ends;
        /// ticks land on nice values inside the range by themselves. Hosts
        /// should call this instead of re-deriving ranges.
        ///
        /// Each series contributes its exact original GPU primitive domain:
        /// valid adjacent segments for line-only mode, paired finite points
        /// for point-bearing modes, and enabled error endpoints. The reduction
        /// is submitted eagerly when data or series metadata changes and cached
        /// by the normalized mode plus role-specific column revisions.
        pub fn auto_fit_all(&mut self, padding: f64) -> js_sys::Promise {
            let snapshot = (|| -> Result<_, JsValue> {
                let token = self
                    .renderer
                    .begin_fit_commit(self.chart_id)
                    .map_err(js_err)?;
                let series = self.chart_series().to_vec();
                let prepared =
                    self.prepare_series_extent_cache(&series, &self.column_revisions, true)?;
                let mut jobs = Vec::new();
                jobs.try_reserve(prepared.extents.len()).map_err(js_err)?;
                jobs.extend(prepared.extents.values().cloned());
                Ok((token, prepared, jobs))
            })();

            let (token, prepared, jobs) = match snapshot {
                Ok(snapshot) => snapshot,
                Err(error) => return js_sys::Promise::reject(&error),
            };
            self.publish_series_extent_cache(prepared);
            let fit_epoch = Rc::clone(&self.fit_epoch);
            let alive = Rc::clone(&self.alive);
            let pending_fit_commit = Rc::clone(&self.pending_fit_commit);
            let Some(expected_epoch) = fit_epoch.get().checked_add(1) else {
                return js_sys::Promise::reject(&js_err("fit epoch counter exhausted"));
            };
            fit_epoch.set(expected_epoch);

            future_to_promise(async move {
                let mut x_ext = FitExtent::EMPTY;
                let mut y_ext = FitExtent::EMPTY;
                for job in jobs {
                    let extent = job.wait().await.map_err(js_err)?;
                    if let Some(extent) = extent {
                        x_ext.union(&extent.x);
                        y_ext.union(&extent.y);
                    }
                }
                if !alive.get() {
                    return Err(js_err("chart was dropped while auto_fit_all was pending"));
                }
                if fit_epoch.get() != expected_epoch {
                    return Err(js_err(
                        "auto_fit_all was superseded by a later data, series, or axis mutation",
                    ));
                }
                let (completion, receiver) = futures_channel::oneshot::channel();
                if let Some(previous) = pending_fit_commit.replace(Some(PendingFitCommit {
                    token,
                    expected_epoch,
                    x_extent: x_ext,
                    y_extent: y_ext,
                    padding,
                    completion,
                })) {
                    let _ = previous
                        .completion
                        .send(Err("auto_fit_all was superseded by a later fit".into()));
                }
                receiver
                    .await
                    .map_err(|_| js_err("auto_fit_all commit channel closed"))?
                    .map_err(js_err)?;
                Ok(JsValue::UNDEFINED)
            })
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
            let old_precise = old_config.draw_style.is_precise();
            let new_precise = new_cfg.draw_style.is_precise();
            let series = self.chart_series().to_vec();
            let next_gpu_picker = if old_precise != new_precise {
                Some(self.build_gpu_picker_for(&series, new_precise)?)
            } else {
                None
            };
            let (_, _, scale) = super::display_config_for_surface(&new_cfg, self.surface_size);
            let mut new_styles = Vec::new();
            new_styles.try_reserve(series.len()).map_err(js_err)?;
            for cfg in &series {
                new_styles.push(self.renderer.create_style_for_series_scaled(cfg, scale));
            }
            self.replace_chart_config(new_cfg)?;
            if let Some(next_gpu_picker) = next_gpu_picker {
                self.gpu_picker = next_gpu_picker;
                self.gpu_picker_dirty = false;
            }
            self.bump_fit_epoch();
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
        pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
            let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
            for cfg in &new_series {
                self.ensure_columns_exist(cfg)?;
            }
            let precise = self.chart_config().draw_style.is_precise();
            let next_gpu_picker = self.build_gpu_picker_for(&new_series, precise)?;
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
            let prepared_extents =
                self.prepare_series_extent_cache(&new_series, &self.column_revisions, false)?;
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
            self.gpu_picker = next_gpu_picker;
            self.gpu_picker_dirty = false;
            self.color_seq = new_len.max(self.color_seq);
            self.bump_fit_epoch();
            self.publish_series_extent_cache(prepared_extents);
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
        /// `"tick_labels_left"`, `"axis_title_left"`, `"legend"`,
        /// `"chart_title"`, …) or `null`. Pure geometry, no selection state
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

        /// Pick the nearest visible data primitive to canvas pixel `(x, y)`.
        /// Scatter hits use the visible marker size, including per-point style
        /// mapping; line strokes snap to the nearest endpoint data point on
        /// the hit segment. Errorbar stems/caps are not pick targets.
        /// Resolves to a JSON string
        /// `{ source_id, series_id, point_index, distance_px }`, or
        /// `undefined` when no visible primitive is within `max_distance_px`.
        /// The `<figgy-chart>` facade parses the string and normalizes
        /// `undefined` to `null`.
        pub fn pick_point(&mut self, x: f32, y: f32, max_distance_px: f32) -> js_sys::Promise {
            if let Err(error) = self.repair_gpu_picker_if_dirty() {
                return js_sys::Promise::reject(&error);
            }
            let (query, display_scale) = super::gpu_pick_query_for_surface(
                self.chart_config(),
                self.surface_size,
                [x, y],
                max_distance_px,
            );
            let ticket = match self.gpu_picker.pick_with_display_scale(
                self.renderer.pool(),
                query,
                display_scale,
            ) {
                Ok(ticket) => ticket,
                Err(error) => return js_sys::Promise::reject(&js_err(error)),
            };

            future_to_promise(async move {
                let Some(picked) = ticket.resolve().await.map_err(js_err)? else {
                    return Ok(JsValue::UNDEFINED);
                };
                let json = crate::picked_point_json_string(&picked).map_err(js_err)?;
                Ok(JsValue::from_str(&json))
            })
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
                if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable()) {
                    if rz.resize_by(&mut config, handle, dx, dy) == NudgeResult::Moved {
                        self.replace_chart_config(config)?;
                    }
                }
                return Ok(());
            }
            if self.dragging
                && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
            {
                if drag.drag_by(&mut config, dx, dy) == NudgeResult::Moved {
                    self.replace_chart_config(config)?;
                }
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

        /// Process pending pool maintenance and draw only when visual state is
        /// dirty. A clean rAF tick returns before touching the pool or surface.
        pub fn frame(&mut self) -> Result<(), JsValue> {
            self.commit_pending_fit();
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
                self.needs_defrag,
            );
            let refresh_raster = match decision {
                FrameDecision::Clean => return Ok(()),
                FrameDecision::MaintenanceOnly => {
                    let _ = self.process_pending_defrag()?;
                    return Ok(());
                }
                FrameDecision::Draw { refresh_raster } => refresh_raster,
            };

            // Rendering preparation owns the internal inactive error lane.
            // It is never scanned or uploaded on a clean or maintenance-only
            // frame.
            self.ensure_zero_column_for_render()?;
            let _ = self.process_pending_defrag()?;
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

            let series_configs = self.chart_series().to_vec();
            let series: Vec<Series<'_>> = self
                .styles
                .iter()
                .zip(series_configs.iter())
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
        pub async fn export_png(&mut self, scale: f32) -> Result<js_sys::Uint8Array, JsValue> {
            self.ensure_zero_column_for_render()?;
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

        /// Load the bundled demo (sine + RC charge curves) — lets a frontend
        /// see a real chart without wiring data first. Repeated calls replace
        /// the demo registrations and keep the same series declarations.
        pub fn load_demo(&mut self) -> Result<(), JsValue> {
            let (xs, ys) = renderer::demo::sine_data(512);
            let (ts, vs) = renderer::demo::rc_data(512);

            self.upsert_column_f64_as_f32("demo_x", &xs)?;
            self.upsert_column_f64_as_f32("demo_sin", &ys)?;
            self.upsert_column_f64_as_f32("demo_t", &ts)?;
            self.upsert_column_f64_as_f32("demo_rc", &vs)?;
            self.add_line_series("sine", "demo_x", "demo_sin", 2.0, "sin(x)")?;
            self.add_line_series("rc", "demo_t", "demo_rc", 2.0, "RC charge")?;
            self.auto_fit_x("demo_x", 0.02)?;
            self.auto_fit_y("demo_sin", 0.10)?;
            self.set_title("figgy")?;
            self.set_x_title("x")?;
            self.set_y_title("y")?;
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
