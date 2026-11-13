//! Histogram, category-bar, distribution and range-bar themes.

use super::super::{ChartKind, Theme};

pub(super) const HISTOGRAM: &[&str] = &["default"];
pub(super) const CATEGORY_BARS: &[&str] = &["default"];
pub(super) const DISTRIBUTION: &[&str] = &["default"];
pub(super) const RANGE_BARS: &[&str] = &["default"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    let _ = (kind, name);
    None
}
