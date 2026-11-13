//! Radial (pie, donut, sunburst) themes.

use super::super::RadialTheme;

pub(super) const NAMES: &[&str] = &["default"];

/// The radial theme `name`, past `default`.
pub(super) fn named(name: &str) -> Option<RadialTheme> {
    let _ = name;
    None
}
