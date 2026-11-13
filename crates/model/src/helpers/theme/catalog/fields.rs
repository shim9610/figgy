//! Field, vector, 3D, mesh, polar, radar, parallel and ternary themes.

use super::super::{ChartKind, Theme};

pub(super) const FIELD: &[&str] = &["default"];
pub(super) const VECTOR_FIELD: &[&str] = &["default"];
pub(super) const SURFACE_3D: &[&str] = &["default"];
pub(super) const TRI_MESH: &[&str] = &["default"];
pub(super) const POLAR: &[&str] = &["default"];
pub(super) const RADAR: &[&str] = &["default"];
pub(super) const PARALLEL: &[&str] = &["default"];
pub(super) const TERNARY: &[&str] = &["default"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    let _ = (kind, name);
    None
}
