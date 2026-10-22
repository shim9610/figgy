//! Decoration renderer. Draws the decoration elements above the data onto
//! the CPU raster [`Canvas`]: tick labels, axis titles, the chart title, the
//! legend, the colourbar and the selection boxes. The axis renderer
//! ([`crate::axis_render`]) owns the raster entry points and the axis geometry
//! these elements are placed against.

use crate::raster::Canvas;

use crate::config::{AxisOptions, AxisScale, LabelStyle};
use crate::format::LabelFormat;
use crate::layout::{DataArea, RectF, Side, label_origin, point_on_rect_side};
use crate::text::{RichSegment, RichText, TextAlign};
use crate::text_render::{
    FontPolicy, draw_plain_text, draw_rich_text, measure_plain_text, measure_rich_text,
};

mod tick_labels;
pub(crate) use tick_labels::*;
mod tick_format;
pub(crate) use tick_format::*;
