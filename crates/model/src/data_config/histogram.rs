//! Histogram bar styles.

use super::*;

/// Bar appearance for [`DataRenderType::Histogram`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarStyleConfig {
    pub fill_color: Color,
    pub border_color: Color,
    pub border_width: f32,
    /// The value bars grow from — usually 0. On a logarithmic count axis 0 has
    /// no position, and the base is clamped to the axis minimum instead.
    pub baseline: f64,
    /// Screen-space gap between neighbouring bars. The renderer reduces the
    /// gap when necessary so a positive-width bin keeps a visible footprint;
    /// Subpixel bins use a GPU pixel-column maximum envelope instead: gap
    /// and positive width ratios do not cut holes in that envelope.
    pub gap_px: f32,
    /// Fraction of each bin occupied by its bar, centred on the bin. Values are
    /// clamped to `0..=1` by the renderer. `1` uses the whole bin before
    /// applying [`Self::gap_px`], while `0.8` leaves 10% empty on each side.
    #[cfg_attr(
        feature = "serde",
        serde(default = "crate::defaults::histogram_bar_width_ratio")
    )]
    pub width_ratio: f32,
    /// Which column is edges and which is counts. This alone decides; the
    /// `edges = counts + 1` length relation is not used to guess.
    pub orientation: BarOrientation,
    /// Sparse, declaration-order overrides keyed by rendered bin index. A
    /// later record for the same index layers over an earlier partial record.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub bar_style_overrides: Option<Vec<DataBarStyleOverride>>,
}

/// Partial appearance override for one histogram bin.
///
/// `None` inherits the series-level value. A zero `border_width` removes the
/// outline for just that bin.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarBinStyleConfig {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub fill_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub border_color: Option<Color>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub border_width: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub gap_px: Option<f32>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub width_ratio: Option<f32>,
}

/// One sparse histogram-bin style override.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DataBarStyleOverride {
    pub index: usize,
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub style: DataBarBinStyleConfig,
}

/// Bar direction, and with it the roles of `x_column` / `y_column`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BarOrientation {
    /// Bars rise along y: `x_column` is edges, `y_column` is counts.
    Vertical,
    /// Bars extend along x: `y_column` is edges, `x_column` is counts.
    Horizontal,
}
