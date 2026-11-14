//! Annotations, layers, decorations and selections.

#![allow(unused_imports)]
// Some sources are read only by `Default` (`crate::helpers`).
#![cfg_attr(not(feature = "helpers"), allow(dead_code))]

use super::*;
use crate::annotation::*;
use crate::decoration::*;
use crate::format::*;
use crate::layer::*;
use crate::layout::*;
use crate::line::*;
use crate::text::*;
use crate::*;

/// The documented default of [`AnnotationLayer`]: the value a document that
/// omits it takes.
pub(crate) fn annotation_layer() -> AnnotationLayer {
    AnnotationLayer {
        visible: true,
        items: Vec::new(),
    }
}

/// The documented default of [`AnnotationLineStyle`]: the value a document that
/// omits it takes.
pub(crate) fn annotation_line_style() -> AnnotationLineStyle {
    AnnotationLineStyle {
        color: Color::from_rgb8(60, 60, 60),
        width: 1.5,
        style: LineStylePreset::Solid,
    }
}

/// The documented default of [`ArrowStyle`]: the value a document that
/// omits it takes.
pub(crate) fn arrow_style() -> ArrowStyle {
    ArrowStyle {
        line: annotation_line_style(),
        head_length: 10.0,
        head_width: 8.0,
    }
}

/// The documented default of [`BoxStyle`]: the value a document that
/// omits it takes.
pub(crate) fn box_style() -> BoxStyle {
    BoxStyle {
        fill: Color::from_rgba8(255, 255, 255, 230),
        border_color: Color::from_rgb8(60, 60, 60),
        border_width: 1.0,
        padding: 4.0,
    }
}

/// The documented default of [`DecorationLayout`]: the value a document that
/// omits it takes.
pub(crate) fn decoration_layout() -> DecorationLayout {
    DecorationLayout {
        data_area: None,
        offsets: Vec::new(),
        selected: None,
    }
}

/// The documented default of [`InsetMarker`]: the value a document that
/// omits it takes.
pub(crate) fn inset_marker() -> InsetMarker {
    InsetMarker {
        window: None,
        line: crate::annotation::AnnotationLineStyle {
            color: crate::color::Color::from_rgb8(120, 128, 140),
            width: 1.0,
            style: crate::line::LineStylePreset::Solid,
        },
        connectors: true,
    }
}

/// The documented default of [`LayerRef`]: the value a document that
/// omits it takes.
pub(crate) const fn layer_ref() -> LayerRef {
    LayerRef::Base
}

/// The documented default of [`NoteAlign`]: the value a document that
/// omits it takes.
pub(crate) const fn note_align() -> NoteAlign {
    NoteAlign::TopLeft
}

/// The documented default of [`OverlayLayer`]: the value a document that
/// omits it takes.
pub(crate) fn overlay_layer() -> OverlayLayer {
    OverlayLayer {
        y_axis: axis_options_y(),
        link_x: true,
        x_axis: axis_options_x(),
        link_y: false,
        series: Vec::new(),
        legend: None,
    }
}

/// The documented default of [`ShapeStyle`]: the value a document that
/// omits it takes.
pub(crate) fn shape_style() -> ShapeStyle {
    ShapeStyle {
        fill: None,
        border: annotation_line_style(),
        below: false,
    }
}

/// The documented default of [`Subplot`]: the value a document that
/// omits it takes.
pub(crate) fn subplot() -> Subplot {
    Subplot {
        config: config(),
        series: Vec::new(),
        link_x: None,
        link_y: None,
        marker: None,
        text_scale: crate::layer::SubplotTextScale::Auto,
    }
}

/// [`Annotation::visible`] where a document omits it.
#[cfg(feature = "serde")]
pub(crate) fn annotation_visible() -> bool {
    true
}
