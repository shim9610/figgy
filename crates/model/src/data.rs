//! Data containers — role-agnostic column storage definitions.
//!
//! Role assignment (X / Y / X-error / Y-error) lives in
//! [`crate::data_config::SeriesConfig`]; `Column` / `DataCell` themselves are
//! role-agnostic. The upload adapter (`ColumnSource`) is the renderer's
//! concern and lives in the renderer crate.

/// Identifier under which a column is registered in a column store (e.g. the
/// renderer's GPU column pool). Plain `String` — interning is the store's job.
pub type ColumnId = String;

/// A bundle of columns belonging to one logical data source.
#[derive(Debug, Clone, PartialEq)]
pub struct DataCell<T> {
    pub data_id: String,
    pub columns: Vec<Column<T>>,
}

/// A single column: data + cached min/max.
///
/// The role (axis or errorbar) is not encoded here — `SeriesConfig.x_column /
/// y_column` and `DataRenderType`'s `ErrorRef` reference columns by id.
#[derive(Debug, Clone, PartialEq)]
pub struct Column<T> {
    pub data: Vec<T>,
    pub min: T,
    pub max: T,
}
