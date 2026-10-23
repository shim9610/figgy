//! The side tags of the decoration stroker.
//!
//! Each layer entry derives one [`DecoStroker`] from `config.draw_style` and
//! the canvas sink threads it down: `Precise` executes the exact pre-stroker
//! code path (plain `draw_line` / `draw_rect`); the sketch arm mixes stable
//! kind+index element tags ("axis_left", "tick_left_3", "grid_major_x_2",
//! "legend_box") into the global seed so every element wobbles differently
//! but identically across re-rasters. The selection overlay and all text
//! stay off the stroker — unconditionally precise.

use super::*;

/// Stable side name used in stroker element tags.
pub(crate) fn side_tag(side: &Side) -> &'static str {
    match side {
        Side::Top => "top",
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    }
}
