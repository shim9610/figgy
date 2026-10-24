//! Decoration renderer. Places the decoration elements above the data
//! through the sink: tick labels, the colourbar's chrome, and the titles,
//! legend and colourbar strip the model places. The axis renderer
//! ([`crate::axis_render`]) owns the raster entry points and the axis geometry
//! these elements are placed against; the canvas sink ([`CanvasSink`]) turns
//! every placed part into pixels.

use crate::raster::{Canvas, Paint};

use crate::config::{AxisOptions, AxisScale, Config, LabelStyle};
use crate::format::LabelFormat;
use crate::layout::{
    DataArea, Element, RectF, Side, axis_offset, label_origin, label_rect, point_on_rect_side,
    visible_colorbar_rect,
};
use crate::select::emit_element;
use crate::sink::{PlacedText, Sink, Slot, StrokeStyle, TextRef};
use crate::text::{MeasureText, RichSegment, RichText, TextAlign};
use crate::text_render::{FontPolicy, measure_plain_text, measure_rich_text};

mod tick_labels;
pub(crate) use tick_labels::*;
mod tick_format;
pub(crate) use tick_format::*;
mod axis_titles;
pub(crate) use axis_titles::*;
mod colorbar;
pub(crate) use colorbar::*;
mod sink;
pub(crate) use sink::CanvasSink;
pub use sink::{draw_selection_box, draw_selection_boxes};
