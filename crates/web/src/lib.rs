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
fn picked_point_json_string(picked: &renderer::PickedPoint) -> serde_json::Result<String> {
    serde_json::to_string(&serde_json::json!({
        "source_id": picked.source_id.as_ref(),
        "series_id": &picked.series_id,
        "point_index": picked.point_index,
        "data_x": picked.data_x,
        "data_y": picked.data_y,
        "distance_px": picked.distance_px,
    }))
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct RevisionedColumn {
    id: String,
    revision: u64,
}

/// Exact GPU extent identity for one value/error association.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ErrorbarExtentKey {
    value: RevisionedColumn,
    lower: RevisionedColumn,
    upper: RevisionedColumn,
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
fn errorbar_extent_key_from(
    revisions: &std::collections::HashMap<String, u64>,
    value_id: &str,
    errors: &renderer::data_config::ErrorRef,
) -> Option<ErrorbarExtentKey> {
    let (lower_id, upper_id) = err_cols(errors);
    Some(ErrorbarExtentKey {
        value: revisioned_column_from(revisions, value_id)?,
        lower: revisioned_column_from(revisions, lower_id)?,
        upper: revisioned_column_from(revisions, upper_id)?,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn active_errorbar_extent_keys(
    series_cfgs: &[renderer::SeriesConfig],
    revisions: &std::collections::HashMap<String, u64>,
) -> Result<Vec<ErrorbarExtentKey>, std::collections::TryReserveError> {
    use std::collections::HashSet;

    let capacity = series_cfgs.len().saturating_mul(2);
    let mut active = HashSet::new();
    active.try_reserve(capacity)?;
    let mut ordered = Vec::new();
    ordered.try_reserve(capacity)?;

    for cfg in series_cfgs {
        let (err_x, err_y) = err_refs(&cfg.render_type);
        for key in [
            err_x.and_then(|errors| errorbar_extent_key_from(revisions, &cfg.x_column, errors)),
            err_y.and_then(|errors| errorbar_extent_key_from(revisions, &cfg.y_column, errors)),
        ]
        .into_iter()
        .flatten()
        {
            if active.insert(key.clone()) {
                ordered.push(key);
            }
        }
    }
    Ok(ordered)
}

#[cfg(any(target_arch = "wasm32", test))]
fn select_errorbar_extent_map<T: Clone>(
    ordered_keys: Vec<ErrorbarExtentKey>,
    existing: &std::collections::HashMap<ErrorbarExtentKey, T>,
    mut make_pending: impl FnMut() -> T,
) -> Result<
    (
        std::collections::HashMap<ErrorbarExtentKey, T>,
        Vec<ErrorbarExtentKey>,
    ),
    std::collections::TryReserveError,
> {
    use std::collections::HashMap;

    let mut selected = HashMap::new();
    selected.try_reserve(ordered_keys.len())?;
    let mut pending = Vec::new();
    pending.try_reserve(ordered_keys.len())?;
    for key in ordered_keys {
        let value = if let Some(value) = existing.get(&key) {
            value.clone()
        } else {
            pending.push(key.clone());
            make_pending()
        };
        selected.insert(key, value);
    }
    Ok((selected, pending))
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
        Color, DataErrorBarStyleConfig, DataRenderType, DataScatterStyleConfig, ScatterShape,
        SeriesConfig,
    };

    use super::{
        ColumnRegistryAction, INTERNAL_ZERO_COLUMN_ID, active_errorbar_extent_keys,
        checked_column_revision, column_update_invalidates_fit, display_config_for_surface,
        errorbar_extent_key_from, fit_display_panel, picked_point_json_string,
        prepare_column_metadata, required_internal_zero_column_len, select_errorbar_extent_map,
        validate_column_data_len, validate_column_registry_action, validate_public_column_id,
    };

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
    fn picked_point_json_includes_data_coordinates() {
        let picked = renderer::PickedPoint {
            source_id: Some("source-a".into()),
            series_id: "series-a".into(),
            point_index: 7,
            data_x: 12.5,
            data_y: -3.25,
            distance_px: 4.0,
        };

        let json: serde_json::Value =
            serde_json::from_str(&picked_point_json_string(&picked).unwrap()).unwrap();

        assert_eq!(json["source_id"], "source-a");
        assert_eq!(json["series_id"], "series-a");
        assert_eq!(json["point_index"], 7);
        assert_eq!(json["data_x"], 12.5);
        assert_eq!(json["data_y"], -3.25);
        assert_eq!(json["distance_px"], 4.0);
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
    fn future_errorbar_map_reuses_only_identical_revision_keys() {
        let symmetric = |id: &str| ErrorRef::Symmetric {
            column: id.to_string(),
        };
        let old_revisions = HashMap::from([
            ("stable-value".to_string(), 1),
            ("stable-error".to_string(), 2),
            ("changed-value".to_string(), 3),
            ("changed-error".to_string(), 4),
        ]);
        let mut future_revisions = old_revisions.clone();
        future_revisions.insert("changed-value".to_string(), 5);
        assert!(
            active_errorbar_extent_keys(&[], &future_revisions)
                .unwrap()
                .is_empty()
        );

        let stable_key = errorbar_extent_key_from(
            &future_revisions,
            "stable-value",
            &symmetric("stable-error"),
        )
        .unwrap();
        let old_changed_key =
            errorbar_extent_key_from(&old_revisions, "changed-value", &symmetric("changed-error"))
                .unwrap();
        let future_changed_key = errorbar_extent_key_from(
            &future_revisions,
            "changed-value",
            &symmetric("changed-error"),
        )
        .unwrap();
        let stable_job = Rc::new(10u8);
        let old_changed_job = Rc::new(20u8);
        let existing = HashMap::from([
            (stable_key.clone(), Rc::clone(&stable_job)),
            (old_changed_key, Rc::clone(&old_changed_job)),
        ]);

        let (selected, pending) = select_errorbar_extent_map(
            vec![stable_key.clone(), future_changed_key.clone()],
            &existing,
            || Rc::new(30u8),
        )
        .unwrap();

        assert!(Rc::ptr_eq(selected.get(&stable_key).unwrap(), &stable_job));
        assert!(!Rc::ptr_eq(
            selected.get(&future_changed_key).unwrap(),
            &old_changed_job
        ));
        assert_eq!(pending, vec![future_changed_key]);
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
    use std::{
        cell::{Cell, RefCell},
        collections::{HashMap, HashSet},
        rc::Rc,
        sync::Arc,
    };

    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::{future_to_promise, spawn_local};
    use web_sys::HtmlCanvasElement;

    use renderer::data_config::ErrorRef;
    use renderer::data_render::ColumnPool;
    use renderer::gpu_pick::{
        GpuPickEngine, GpuPickQuery, GpuPickSeriesReplacement, PreparedGpuPickSeriesBatch,
    };
    use renderer::layout::{ChartArea, Rect};
    use renderer::line::LineStylePreset;
    use renderer::text::{RichText, rich_segments_from_text};
    use renderer::{
        Chart, ChartDrawItem, ChartStyle, ChartView, Color, ColumnSource, CpuTextMeasure,
        DataLineStyleConfig, DataRenderType, DefragPolicy, FitExtent, HiLoColumnSource, HitId,
        HitMap, Renderer, ResizeHandle as ModelResizeHandle, SelectionBox, Series, SeriesConfig,
        WindowedRenderer,
    };

    use crate::borrowed_column::{
        BorrowedCastF32Column, BorrowedF32Column, BorrowedF64Column, ZeroColumnSource,
    };
    use crate::gpu_pick_style::OwnedGpuPickSeriesDescriptor;
    use crate::scalar_job::ScalarExtentJob;
    use crate::{
        ColumnRegistryAction, ErrorbarExtentKey, INTERNAL_ZERO_COLUMN_ID,
        active_errorbar_extent_keys, column_update_invalidates_fit, err_refs,
        errorbar_extent_key_from, prepare_column_metadata, required_internal_zero_column_len,
        select_errorbar_extent_map, validate_column_data_len, validate_column_registry_action,
        validate_public_column_id,
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
        gpu_picker: GpuPickEngine,
        gpu_picker_dirty: bool,
        chart: Rc<RefCell<Chart>>,
        view: ChartView,
        /// Current WebGPU surface size in physical canvas pixels. This is a
        /// viewport property, not the exported document size.
        surface_size: (u32, u32),
        series_cfgs: Vec<SeriesConfig>,
        styles: Vec<ChartStyle>,
        /// 1:1 with `series_cfgs` — legend label text (None = no legend row).
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
        /// Already-submitted GPU errorbar reductions, keyed only by the three
        /// current column revisions. Each job retains one scalar extent.
        errorbar_extents: HashMap<ErrorbarExtentKey, Rc<ScalarExtentJob>>,
        /// Invalidates an async fit if a later data/series/range mutation wins.
        fit_epoch: Rc<Cell<u64>>,
        alive: Rc<Cell<bool>>,
        /// A removal happened — defragment once on the next frame.
        needs_defrag: bool,
        /// Monotonic color assignment for newly registered series.
        color_seq: usize,
        hitmap: HitMap,
        selected: Option<HitId>,
        dragging: bool,
        resizing: Option<ModelResizeHandle>,
        cycle: renderer::ColorCycle,
        clear_color: Color,
        view_dirty: bool,
    }

    impl FiggyChart {
        fn display_scale_and_panel(&self) -> (f32, Rect) {
            let logical = self.chart.borrow().config().chart_area.0;
            super::fit_display_panel((logical.width, logical.height), self.surface_size)
        }

        fn display_config(&self) -> (renderer::config::Config, Rect, f32) {
            let chart = self.chart.borrow();
            super::display_config_for_surface(chart.config(), self.surface_size)
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
            let chart = self.chart.borrow();
            let content = &chart.config().legend.content;
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
            self.series_cfgs
                .get(i)
                .and_then(|cfg| cfg.label.clone())
                .or_else(|| self.labels.get(i)?.as_ref().map(|s| self.label_text(s)))
        }

        fn append_legend_entry_for(&mut self, i: usize) {
            let Some(cfg) = self.series_cfgs.get(i).cloned() else {
                return;
            };
            let Some(label) = self.fallback_label(i) else {
                return;
            };
            self.chart.borrow_mut().with_decoration_change(|c| {
                renderer::config::append_legend_entry_rich(
                    &mut c.legend.content,
                    renderer::config::series_symbol_segments(&cfg),
                    label.segments,
                );
                c.legend.visible = true;
            });
        }

        fn set_legend_entry_for(&mut self, i: usize, label: RichText) {
            let Some(cfg) = self.series_cfgs.get(i).cloned() else {
                return;
            };
            self.chart.borrow_mut().with_decoration_change(|c| {
                renderer::config::set_legend_entry_label(
                    &mut c.legend.content,
                    i,
                    renderer::config::series_symbol_segments(&cfg),
                    label.segments,
                );
                c.legend.visible = true;
            });
        }

        fn remove_legend_entry_for(&mut self, i: usize) {
            self.chart.borrow_mut().with_decoration_change(|c| {
                if renderer::config::remove_legend_entry(&mut c.legend.content, i)
                    && c.legend.content.segments.is_empty()
                {
                    c.legend.visible = false;
                }
            });
        }

        fn sync_legend_symbols(&mut self, append_missing: bool) {
            let existing = {
                let chart = self.chart.borrow();
                renderer::config::legend_entry_count(&chart.config().legend.content)
            };
            self.chart.borrow_mut().with_decoration_change(|c| {
                renderer::config::update_legend_symbols_preserving_text(
                    &mut c.legend.content,
                    &self.series_cfgs,
                );
            });
            if append_missing {
                if existing > self.series_cfgs.len() {
                    for i in (self.series_cfgs.len()..existing).rev() {
                        self.remove_legend_entry_for(i);
                    }
                }
                for i in existing..self.series_cfgs.len() {
                    self.append_legend_entry_for(i);
                }
            }
        }

        /// Explicit reset: rebuild every auto legend row from SeriesConfig.label
        /// first, falling back to the wrapper's legacy string labels.
        fn rebuild_legend_from_series_labels(&mut self) {
            let entries: Vec<(SeriesConfig, RichText)> = self
                .series_cfgs
                .iter()
                .cloned()
                .enumerate()
                .filter_map(|(i, cfg)| Some((cfg, self.fallback_label(i)?)))
                .collect();

            self.chart.borrow_mut().with_decoration_change(|c| {
                c.legend.content.segments.clear();
                for (cfg, label) in entries {
                    renderer::config::append_legend_entry_rich(
                        &mut c.legend.content,
                        renderer::config::series_symbol_segments(&cfg),
                        label.segments,
                    );
                }
                c.legend.visible = !c.legend.content.segments.is_empty();
            });
            self.legend_auto_managed = true;
        }

        fn rebuild_styles(&mut self) {
            let (scale, _) = self.display_scale_and_panel();
            self.styles = self
                .series_cfgs
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

        fn gpu_pick_index_before(&self, logical_index: usize) -> usize {
            self.series_cfgs[..logical_index]
                .iter()
                .filter(|cfg| self.pick_series_active_in_metadata(cfg))
                .count()
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
                let mut descriptors = Vec::new();
                descriptors
                    .try_reserve(self.series_cfgs.len())
                    .map_err(js_err)?;
                descriptors.extend(
                    self.series_cfgs
                        .iter()
                        .map(|cfg| OwnedGpuPickSeriesDescriptor::from_series(cfg, precise)),
                );
                return Ok(PickerUploadPlan::Rebuild(descriptors));
            }

            let mut replacements = Vec::new();
            replacements
                .try_reserve(self.series_cfgs.len())
                .map_err(js_err)?;
            let mut gpu_index = 0usize;
            for cfg in &self.series_cfgs {
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
            let precise = self.chart.borrow().config().draw_style.is_precise();
            let picker = self.build_gpu_picker_for(&self.series_cfgs, precise)?;
            self.gpu_picker = picker;
            self.gpu_picker_dirty = false;
            Ok(())
        }

        fn update_gpu_picker_series(
            &mut self,
            existing: Option<usize>,
            cfg: &SeriesConfig,
        ) -> Result<(), JsValue> {
            let logical_index = existing.unwrap_or(self.series_cfgs.len());
            let gpu_index = self.gpu_pick_index_before(logical_index);
            let old_active = existing
                .is_some_and(|index| self.pick_series_active_in_metadata(&self.series_cfgs[index]));
            let new_active = self.pick_series_active_in_pool(cfg);
            let precise = self.chart.borrow().config().draw_style.is_precise();
            let owned = OwnedGpuPickSeriesDescriptor::from_series(cfg, precise);
            let pool = self.renderer.pool();
            let picker = &mut self.gpu_picker;

            match (old_active, new_active) {
                (true, true) => owned
                    .with_descriptor(|descriptor| {
                        picker.replace_series_at(gpu_index, pool, descriptor)
                    })
                    .map(|_| ())
                    .map_err(js_err),
                (true, false) => picker.remove_series_at(gpu_index).map_err(js_err),
                (false, true) => owned
                    .with_descriptor(|descriptor| {
                        picker.insert_series_at(gpu_index, pool, descriptor)
                    })
                    .map(|_| ())
                    .map_err(js_err),
                (false, false) => Ok(()),
            }
        }

        /// Invalidate an in-flight fit when a later mutation wins call order.
        fn bump_fit_epoch(&self) {
            self.fit_epoch.set(self.fit_epoch.get().wrapping_add(1));
        }

        fn errorbar_extent_key(
            &self,
            value_id: &str,
            errors: &ErrorRef,
        ) -> Option<ErrorbarExtentKey> {
            errorbar_extent_key_from(&self.column_revisions, value_id, errors)
        }

        fn errorbar_job(
            &self,
            value_id: &str,
            errors: &ErrorRef,
        ) -> Result<Rc<ScalarExtentJob>, JsValue> {
            let key = self
                .errorbar_extent_key(value_id, errors)
                .ok_or_else(|| js_err("errorbar references a column without a live revision"))?;
            self.errorbar_extents.get(&key).cloned().ok_or_else(|| {
                js_err("errorbar GPU extent was not submitted at data/series mutation time")
            })
        }

        fn reconcile_errorbar_extents(&mut self) {
            let mut active = HashSet::new();
            let mut ordered = Vec::new();
            for cfg in &self.series_cfgs {
                let (err_x, err_y) = err_refs(&cfg.render_type);
                for key in [
                    err_x.and_then(|errors| self.errorbar_extent_key(&cfg.x_column, errors)),
                    err_y.and_then(|errors| self.errorbar_extent_key(&cfg.y_column, errors)),
                ]
                .into_iter()
                .flatten()
                {
                    if active.insert(key.clone()) {
                        ordered.push(key);
                    }
                }
            }

            self.errorbar_extents.retain(|key, _| active.contains(key));
            for key in ordered {
                if self.errorbar_extents.contains_key(&key) {
                    continue;
                }
                let job = ScalarExtentJob::pending();
                let ticket = self.renderer.begin_errorbar_extent(
                    &key.value.id,
                    &key.lower.id,
                    &key.upper.id,
                );
                self.errorbar_extents.insert(key, Rc::clone(&job));
                match ticket {
                    Ok(ticket) => spawn_local(async move {
                        let result = ticket
                            .resolve()
                            .await
                            .map(|extent| {
                                extent.map(|extent| FitExtent {
                                    min: extent.min,
                                    max: extent.max,
                                    min_positive: extent.min_positive,
                                })
                            })
                            .map_err(|error| error.to_string());
                        job.complete(result);
                    }),
                    Err(error) => job.complete(Err(error.to_string())),
                }
            }
        }

        fn spawn_errorbar_extent(
            ticket: renderer::GpuErrorbarExtentTicket,
            job: Rc<ScalarExtentJob>,
        ) {
            spawn_local(async move {
                let result = ticket
                    .resolve()
                    .await
                    .map(|extent| {
                        extent.map(|extent| FitExtent {
                            min: extent.min,
                            max: extent.max,
                            min_positive: extent.min_positive,
                        })
                    })
                    .map_err(|error| error.to_string());
                job.complete(result);
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
            let ordered_keys = active_errorbar_extent_keys(&self.series_cfgs, &metadata.revisions)
                .map_err(js_err)?;
            let (future_errorbar_extents, pending_errorbars) = select_errorbar_extent_map(
                ordered_keys,
                &self.errorbar_extents,
                ScalarExtentJob::pending,
            )
            .map_err(js_err)?;
            let precise = self.chart.borrow().config().draw_style.is_precise();
            let picker_plan = self.prepare_picker_upload_plan(id, precise)?;
            let device = Arc::clone(self.renderer.device());
            let queue = Arc::clone(self.renderer.queue());
            let mut ticket_jobs = Vec::new();
            ticket_jobs
                .try_reserve(pending_errorbars.len())
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

            for key in &pending_errorbars {
                let ticket = guard
                    .begin_errorbar_extent(&key.value.id, &key.lower.id, &key.upper.id)
                    .map_err(js_err)?;
                let job = Rc::clone(
                    future_errorbar_extents
                        .get(key)
                        .expect("pending errorbar key must have a prepared job"),
                );
                ticket_jobs.push((ticket, job));
            }

            guard.commit();
            match prepared_picker {
                PreparedColumnPicker::Rebuild(picker) => self.gpu_picker = picker,
                PreparedColumnPicker::Batch(batch) => self.gpu_picker.commit_series_batch(batch),
            }
            self.gpu_picker_dirty = false;
            self.columns = metadata.columns;
            self.column_revisions = metadata.revisions;
            self.errorbar_extents = future_errorbar_extents;
            self.next_column_revision = metadata.next_revision;
            self.needs_defrag |= replaced_existing;
            if column_update_invalidates_fit(id) {
                self.bump_fit_epoch();
            }
            for (ticket, job) in ticket_jobs {
                Self::spawn_errorbar_extent(ticket, job);
            }
            Ok(())
        }

        fn upsert_zero_column(&mut self, len: usize) -> Result<(), JsValue> {
            if self.columns.get(INTERNAL_ZERO_COLUMN_ID) == Some(&len) {
                return Ok(());
            }
            let source = ZeroColumnSource::new(len);
            self.upsert_column_atomic(
                INTERNAL_ZERO_COLUMN_ID,
                len,
                ColumnUploadSource::Scalar(&source),
            )
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
            let needed = required_internal_zero_column_len(&self.columns, &self.series_cfgs);
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
    }

    impl Drop for FiggyChart {
        fn drop(&mut self) {
            self.alive.set(false);
            self.bump_fit_epoch();
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
                gpu_picker,
                gpu_picker_dirty: false,
                chart: Rc::new(RefCell::new(chart)),
                view,
                surface_size: (w, h),
                series_cfgs: Vec::new(),
                styles: Vec::new(),
                labels: Vec::new(),
                legend_auto_managed: true,
                columns: HashMap::new(),
                column_revisions: HashMap::new(),
                next_column_revision: 1,
                errorbar_extents: HashMap::new(),
                fit_epoch: Rc::new(Cell::new(0)),
                alive: Rc::new(Cell::new(true)),
                needs_defrag: false,
                color_seq: 0,
                hitmap: HitMap::standard_chart(),
                selected: None,
                dragging: false,
                resizing: None,
                cycle: renderer::ColorCycle::Classic,
                clear_color: Color::WHITE,
                view_dirty: false,
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
            self.chart.borrow_mut().with_decoration_change(|_| {});
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

        /// Unregister a column. Series referencing it are removed too (with
        /// their legend rows) so the chart can never point at freed data.
        /// Returns `true` when the column existed.
        pub fn remove_column(&mut self, id: &str) -> Result<bool, JsValue> {
            validate_public_column_id(id)
                .map_err(|reason| js_err(format!("column '{id}' {reason}")))?;
            if !self.columns.contains_key(id) {
                return Ok(false);
            }
            if !self.renderer.remove_column(id).map_err(js_err)? {
                return Ok(false);
            }
            let was_dirty = self.gpu_picker_dirty;
            let keep: Vec<bool> = self
                .series_cfgs
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
                    if self.pick_series_active_in_metadata(&self.series_cfgs[logical_index]) {
                        let gpu_index = self.gpu_pick_index_before(logical_index);
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
                self.series_cfgs.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.styles.retain(|_| *it.next().unwrap());
                let mut it = keep.iter();
                self.labels.retain(|_| *it.next().unwrap());
                if self.legend_auto_managed {
                    for i in removed.into_iter().rev() {
                        self.remove_legend_entry_for(i);
                    }
                } else {
                    self.sync_legend_symbols(false);
                }
            }
            if self.gpu_picker_dirty {
                let _ = self.rebuild_gpu_picker_now();
            }
            self.bump_fit_epoch();
            self.reconcile_errorbar_extents();
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
            let existing = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id);
            let color = match existing {
                Some(i) => match &self.series_cfgs[i].render_type {
                    DataRenderType::Line { line } => line.line_color,
                    _ => self.cycle.color(i),
                },
                None => {
                    let c = self.cycle.color(self.color_seq);
                    self.color_seq += 1;
                    c
                }
            };
            let label_changed = !label.is_empty();
            let rich_label = if label_changed {
                Some(self.label_text(label))
            } else {
                existing.and_then(|i| self.series_cfgs[i].label.clone())
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
            let style = self.renderer.create_style_for_series_scaled(&cfg, scale);
            if self.gpu_picker_dirty {
                self.gpu_picker.clear_series();
                let mut proposed = self.series_cfgs.clone();
                if let Some(index) = existing {
                    proposed[index] = cfg.clone();
                } else {
                    proposed.push(cfg.clone());
                }
                let precise = self.chart.borrow().config().draw_style.is_precise();
                self.gpu_picker = self.build_gpu_picker_for(&proposed, precise)?;
                self.gpu_picker_dirty = false;
            } else {
                self.update_gpu_picker_series(existing, &cfg)?;
            }

            match existing {
                Some(i) => {
                    self.series_cfgs[i] = cfg;
                    self.styles[i] = style;
                    self.labels[i] = plain_label;
                    if label_changed && let Some(label) = rich_label {
                        self.set_legend_entry_for(i, label);
                    } else {
                        self.sync_legend_symbols(false);
                    }
                }
                None => {
                    self.series_cfgs.push(cfg);
                    self.styles.push(style);
                    self.labels.push(plain_label);
                    if label_changed && rich_label.is_some() {
                        self.append_legend_entry_for(self.series_cfgs.len() - 1);
                    }
                }
            }
            self.bump_fit_epoch();
            self.reconcile_errorbar_extents();
            Ok(())
        }

        /// Set / change / remove a series' legend label. `'\n'` breaks lines;
        /// unicode sub/superscripts (`₀`, `⁻`, …) map to styled segments.
        /// Empty string removes the legend row. Returns `true` when the
        /// series exists.
        pub fn set_series_label(&mut self, series_id: &str, label: &str) -> bool {
            let Some(i) = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id)
            else {
                return false;
            };
            if label.is_empty() {
                self.labels[i] = None;
                self.series_cfgs[i].label = None;
                self.remove_legend_entry_for(i);
            } else {
                let label = self.label_text(label);
                self.labels[i] = Some(Self::rich_text_to_plain(&label));
                self.series_cfgs[i].label = Some(label.clone());
                self.set_legend_entry_for(i, label);
            }
            true
        }

        /// Unregister a series (and its legend row). Columns stay registered.
        /// Returns `true` when the series existed.
        pub fn remove_series(&mut self, series_id: &str) -> bool {
            let Some(i) = self
                .series_cfgs
                .iter()
                .position(|c| c.series_id == series_id)
            else {
                return false;
            };
            let was_dirty = self.gpu_picker_dirty;
            if !was_dirty && self.pick_series_active_in_metadata(&self.series_cfgs[i]) {
                let gpu_index = self.gpu_pick_index_before(i);
                if self.gpu_picker.remove_series_at(gpu_index).is_err() {
                    return false;
                }
            }
            self.series_cfgs.remove(i);
            self.styles.remove(i);
            self.labels.remove(i);
            if self.legend_auto_managed {
                self.remove_legend_entry_for(i);
            } else {
                self.sync_legend_symbols(false);
            }
            if was_dirty {
                let _ = self.rebuild_gpu_picker_now();
            }
            self.bump_fit_epoch();
            self.reconcile_errorbar_extents();
            true
        }

        /// Fit the x axis to a column's range with proportional padding.
        pub fn auto_fit_x(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            let result = self
                .chart
                .borrow_mut()
                .auto_fit_x(self.renderer.pool(), column, padding)
                .map_err(js_err);
            if result.is_ok() {
                self.bump_fit_epoch();
            }
            result
        }

        pub fn auto_fit_y(&mut self, column: &str, padding: f64) -> Result<(), JsValue> {
            let result = self
                .chart
                .borrow_mut()
                .auto_fit_y(self.renderer.pool(), column, padding)
                .map_err(js_err);
            if result.is_ok() {
                self.bump_fit_epoch();
            }
            result
        }

        /// Fit BOTH axes to the union of every registered series, leaving a
        /// uniform `padding` fraction of the data span as margin on each
        /// side (`0.0` = exact fit, `0.05` = 5% top/bottom/left/right).
        /// This is the whole fit policy — no rounding of the range ends;
        /// ticks land on nice values inside the range by themselves. Hosts
        /// should call this instead of re-deriving ranges.
        ///
        /// Errorbar series contribute their full bar extents
        /// (`value − err_lo .. value + err_hi`, the exact GPU arithmetic),
        /// so caps never clip against an auto-fitted range. That pairwise
        /// pass runs once per (series, data) and is cached — repeat fits
        /// stay metadata-cheap.
        pub fn auto_fit_all(&self, padding: f64) -> js_sys::Promise {
            let snapshot = (|| -> Result<_, JsValue> {
                let mut x_ext = FitExtent::EMPTY;
                let mut y_ext = FitExtent::EMPTY;
                let mut jobs = Vec::new();
                for cfg in &self.series_cfgs {
                    let pool = self.renderer.pool();
                    x_ext.union(&Chart::slot_extent(pool, &cfg.x_column).map_err(js_err)?);
                    y_ext.union(&Chart::slot_extent(pool, &cfg.y_column).map_err(js_err)?);
                    let (err_x, err_y) = err_refs(&cfg.render_type);
                    if let Some(errors) = err_x {
                        jobs.push((true, self.errorbar_job(&cfg.x_column, errors)?));
                    }
                    if let Some(errors) = err_y {
                        jobs.push((false, self.errorbar_job(&cfg.y_column, errors)?));
                    }
                }
                Ok((x_ext, y_ext, jobs))
            })();

            let (mut x_ext, mut y_ext, jobs) = match snapshot {
                Ok(snapshot) => snapshot,
                Err(error) => return js_sys::Promise::reject(&error),
            };
            let chart = Rc::clone(&self.chart);
            let fit_epoch = Rc::clone(&self.fit_epoch);
            let alive = Rc::clone(&self.alive);
            let expected_epoch = fit_epoch.get().wrapping_add(1);
            fit_epoch.set(expected_epoch);

            future_to_promise(async move {
                for (is_x, job) in jobs {
                    let extent = job.wait().await.map_err(js_err)?;
                    if let Some(extent) = extent {
                        if is_x {
                            x_ext.union(&extent);
                        } else {
                            y_ext.union(&extent);
                        }
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
                let mut chart = chart
                    .try_borrow_mut()
                    .map_err(|_| js_err("chart is busy applying another synchronous mutation"))?;
                chart.auto_fit_x_extent(&x_ext, padding);
                chart.auto_fit_y_extent(&y_ext, padding);
                Ok(JsValue::UNDEFINED)
            })
        }

        // ---- titles ----

        pub fn set_title(&mut self, text: &str) {
            self.chart.borrow_mut().with_decoration_change(|c| {
                c.chart_title.text.segments = rich_segments_from_text(text);
            });
        }

        pub fn set_x_title(&mut self, text: &str) {
            self.chart.borrow_mut().with_decoration_change(|c| {
                c.bottom_x.title_option.text.segments = rich_segments_from_text(text);
            });
        }

        pub fn set_y_title(&mut self, text: &str) {
            self.chart.borrow_mut().with_decoration_change(|c| {
                c.left_y.title_option.text.segments = rich_segments_from_text(text);
            });
        }

        // ---- presets ----

        /// Apply an axis frame preset to all four axes (decoration-only).
        pub fn apply_axis_preset(&mut self, preset: AxisPreset) {
            let p: renderer::AxisPreset = preset.into();
            self.chart
                .borrow_mut()
                .with_decoration_change(|c| c.apply_axis_preset(p));
        }

        /// Switch the series color rotation: recolors every series in order,
        /// rebuilds their GPU styles, and keeps legend swatches in sync.
        pub fn apply_color_cycle(&mut self, cycle: ColorCycle) {
            self.cycle = cycle.into();
            for (i, cfg) in self.series_cfgs.iter_mut().enumerate() {
                self.cycle.apply_to_series(cfg, i);
            }
            self.color_seq = self.series_cfgs.len();
            self.rebuild_styles();
            self.sync_legend_symbols(false);
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
            serde_json::to_string(self.chart.borrow().config()).map_err(js_err)
        }

        /// Replace the whole option SSoT from JSON. Marks everything dirty —
        /// the next `frame()` re-rasters the chrome and refreshes the
        /// transform, exactly like any other config edit.
        /// JS: `chart.set_config(JSON.stringify(cfg));`
        pub fn set_config(&mut self, json: &str) -> Result<(), JsValue> {
            let new_cfg: renderer::Config = serde_json::from_str(json).map_err(js_err)?;
            let (legend_content_changed, old_precise) = {
                let chart = self.chart.borrow();
                (
                    chart.config().legend.content != new_cfg.legend.content,
                    chart.config().draw_style.is_precise(),
                )
            };
            let new_precise = new_cfg.draw_style.is_precise();
            let next_gpu_picker = if old_precise != new_precise {
                if self.gpu_picker_dirty {
                    self.gpu_picker.clear_series();
                }
                Some(self.build_gpu_picker_for(&self.series_cfgs, new_precise)?)
            } else {
                None
            };
            *self.chart.borrow_mut().config_mut() = new_cfg;
            if let Some(next_gpu_picker) = next_gpu_picker {
                self.gpu_picker = next_gpu_picker;
                self.gpu_picker_dirty = false;
            }
            self.bump_fit_epoch();
            if legend_content_changed {
                self.legend_auto_managed = false;
            }
            self.view_dirty = true;
            self.rebuild_styles();
            Ok(())
        }

        /// Serialize the series declarations (columns, render type, styles).
        pub fn get_series(&self) -> Result<String, JsValue> {
            serde_json::to_string(&self.series_cfgs).map_err(js_err)
        }

        /// Replace the series declarations from JSON. Column references must
        /// already be registered; GPU styles are rebuilt, legend labels are
        /// kept for series ids that survive.
        pub fn set_series(&mut self, json: &str) -> Result<(), JsValue> {
            let new_series: Vec<SeriesConfig> = serde_json::from_str(json).map_err(js_err)?;
            for cfg in &new_series {
                self.ensure_columns_exist(cfg)?;
            }
            if self.gpu_picker_dirty {
                self.gpu_picker.clear_series();
            }
            let precise = self.chart.borrow().config().draw_style.is_precise();
            let next_gpu_picker = self.build_gpu_picker_for(&new_series, precise)?;
            let new_labels = new_series
                .iter()
                .map(|cfg| {
                    cfg.label
                        .as_ref()
                        .map(Self::rich_text_to_plain)
                        .or_else(|| {
                            self.series_cfgs
                                .iter()
                                .position(|old| old.series_id == cfg.series_id)
                                .and_then(|i| self.labels[i].clone())
                        })
                })
                .collect();
            self.labels = new_labels;
            self.series_cfgs = new_series;
            self.gpu_picker = next_gpu_picker;
            self.gpu_picker_dirty = false;
            self.color_seq = self.series_cfgs.len().max(self.color_seq);
            self.bump_fit_epoch();
            self.reconcile_errorbar_extents();
            self.rebuild_styles();
            self.sync_legend_symbols(self.legend_auto_managed);
            Ok(())
        }

        /// Explicitly rebuild the auto legend from `SeriesConfig.label`.
        /// Legacy string labels are used only when a series has no rich label.
        pub fn reset_legend_from_series_labels(&mut self) {
            self.rebuild_legend_from_series_labels();
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
        /// Returns JSON `{ source_id, series_id, point_index, data_x, data_y,
        /// distance_px }`, or `null` when no visible primitive is within
        /// `max_distance_px`.
        pub fn pick_point(&mut self, x: f32, y: f32, max_distance_px: f32) -> js_sys::Promise {
            if let Err(error) = self.repair_gpu_picker_if_dirty() {
                return js_sys::Promise::reject(&error);
            }
            let (display_config, _, _) = self.display_config();
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
            let query = GpuPickQuery {
                transform: renderer::data_render::scatter_transform_from_config(&display_config),
                chart_rect_px: [
                    chart_rect.x as f32,
                    chart_rect.y as f32,
                    chart_rect.width as f32,
                    chart_rect.height as f32,
                ],
                data_area_px,
                canvas_position_px: [x, y],
                max_distance_px,
            };
            let ticket = match self.gpu_picker.pick(self.renderer.pool(), query) {
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
            self.chart.borrow_mut().with_decoration_change(|c| {
                c.picked_points = picked;
            });
            Ok(())
        }

        /// Pointer press. Returns `true` while something is selected.
        /// The host can mirror that state in its own UI.
        pub fn on_press(&mut self, x: f32, y: f32) -> bool {
            let (display_config, _, _) = self.display_config();
            // Resize handles on the selected element win over hit-testing.
            if let Some(id) = self.selected
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
                return true;
            }
            self.resizing = None;

            let new_sel = self.hitmap.hit_test(
                &display_config,
                &CpuTextMeasure::for_style(&display_config.draw_style),
                x,
                y,
            );
            self.dragging = new_sel.is_some_and(|id| {
                self.hitmap
                    .get(id)
                    .is_some_and(|el| el.as_draggable().is_some())
            });
            if new_sel != self.selected {
                self.selected = new_sel;
                self.chart.borrow_mut().with_decoration_change(|_| {});
            }
            self.selected.is_some()
        }

        /// Pointer move with frame delta — drags or resizes the selection.
        pub fn on_move(&mut self, dx: f32, dy: f32) {
            let Some(id) = self.selected else { return };
            let (dx, dy) = self.display_delta_to_document(dx, dy);
            if let Some(handle) = self.resizing {
                if let Some(rz) = self.hitmap.get(id).and_then(|el| el.as_resizable()) {
                    let _ = rz.resize_by(self.chart.borrow_mut().config_mut(), handle, dx, dy);
                }
                return;
            }
            if self.dragging
                && let Some(drag) = self.hitmap.get(id).and_then(|el| el.as_draggable())
            {
                let _ = drag.drag_by(self.chart.borrow_mut().config_mut(), dx, dy);
            }
        }

        pub fn on_release(&mut self) {
            self.dragging = false;
            self.resizing = None;
        }

        pub fn has_selection(&self) -> bool {
            self.selected.is_some()
        }

        // ---- frame / resize / export ----

        /// Process pending pool maintenance + dirty flags, then draw one
        /// frame. Call from `requestAnimationFrame`; with nothing dirty the
        /// raster cost is skipped and only the GPU pass runs.
        pub fn frame(&mut self) -> Result<(), JsValue> {
            self.ensure_zero_column_for_render()?;

            // Coalesced auto-defrag: removals since the last frame collapse
            // into one compaction pass (GPU-internal copies only).
            if self.needs_defrag {
                self.needs_defrag = false;
                let relocated = self.renderer.defragment().map_err(js_err)?;
                if relocated && !self.gpu_picker_dirty {
                    self.gpu_picker
                        .rebind_columns(self.renderer.pool())
                        .map_err(js_err)?;
                }
            }

            // Browser resize is preview zoom, not a document mutation. The
            // stored chart remains the export SSoT; the live canvas renders a
            // scaled/letterboxed display chart derived from it.
            let (display_config, panel_rect, _) = self.display_config();
            let display_chart = Chart::new(display_config);
            let view_dirty = self.view_dirty;
            let raster_dirty = {
                let mut chart = self.chart.borrow_mut();
                let raster_dirty = chart.consume_raster_dirty();
                // `Renderer::prepare` (inside `draw` below) rewrites the transform
                // uniform from the current config every frame; the data-dirty flag
                // only needs resetting.
                let _ = chart.consume_data_dirty();
                raster_dirty
            };
            if view_dirty || raster_dirty {
                let sel_boxes: Vec<SelectionBox> = self
                    .selected
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
                self.view_dirty = false;
            }

            let series: Vec<Series<'_>> = self
                .series_cfgs
                .iter()
                .zip(self.styles.iter())
                .map(|(cfg, style)| Series { config: cfg, style })
                .collect();
            let items = [ChartDrawItem {
                view: &self.view,
                chart_config: display_chart.config(),
                series: &series,
            }];
            self.renderer.draw(self.clear_color, &items).map_err(js_err)
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
            let export_chart = {
                let chart = self.chart.borrow();
                Chart::new(chart.config().clone())
            };
            let bytes = self
                .renderer
                .export_panel_png_bytes_with_clear_async(
                    &export_chart,
                    &self.series_cfgs,
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
            self.set_title("figgy");
            self.set_x_title("x");
            self.set_y_title("y");
            Ok(())
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::*;
