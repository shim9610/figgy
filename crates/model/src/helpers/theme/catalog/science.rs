//! Energy-level and Feynman themes.

use super::super::{ChartKind, Theme};

pub(super) const ENERGY_LEVELS: &[&str] = &["default"];
pub(super) const FEYNMAN: &[&str] = &["default"];

/// The theme `name` of `kind`'s catalogue, past `default`.
pub(super) fn named(kind: ChartKind, name: &str) -> Option<Theme> {
    let _ = (kind, name);
    None
}
