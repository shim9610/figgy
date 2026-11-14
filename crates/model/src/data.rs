//! Data containers — role-agnostic column storage definitions.
//!
//! Role assignment (X / Y / X-error / Y-error) lives in
//! [`crate::data_config::SeriesConfig`]; `Column` itself is role-agnostic.
//! The upload adapter (`ColumnSource`) is the renderer's concern and lives in
//! the renderer crate.

/// Identifier under which a column is registered in a column store (e.g. the
/// renderer's GPU column pool). Plain `String` — interning is the store's job.
pub type ColumnId = String;

/// A single column of owned values: one ready-made implementation of the
/// renderer's `ColumnSource`. A host may implement that trait over its own
/// storage instead. The renderer measures the statistics (minimum, maximum,
/// smallest positive) while it uploads the values, so none is stored here.
///
/// The role (axis or errorbar) is not encoded here — `SeriesConfig.x_column /
/// y_column` and `DataRenderType`'s `ErrorRef` reference columns by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Column<T> {
    pub data: Vec<T>,
}
