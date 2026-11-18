//! Decoration renderer. Places the decoration elements above the data
//! through the sink: tick labels, the colourbar's chrome, and the titles,
//! legend and colourbar strip the model places. The axis renderer
//! ([`crate::axis_render`]) owns the raster entry points and the axis geometry
//! these elements are placed against; the canvas sink ([`CanvasSink`]) turns
//! every placed part into pixels.

use crate::raster::{Canvas, Paint};

use crate::layout::{RectF, Side};

pub(crate) use crate::chrome::*;
mod colorbar;
pub(crate) use colorbar::*;
mod sink;
#[cfg(test)]
mod tick_format_tests;
pub(crate) use sink::CanvasSink;
pub use sink::{draw_selection_box, draw_selection_boxes};
