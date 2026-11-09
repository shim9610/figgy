//! Constructors for the parts a host adds to a chart (#265): a colour with
//! another alpha, annotation items, a right-hand overlay axis and a subplot.
//!
//! Each builds the part's documented default and then assigns the fields it
//! names, so a field added to a type later takes its default here too.

use crate::annotation::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle, BoxStyle, NoteAlign,
};
use crate::color::Color;
use crate::config::{AxisOptions, Config};
use crate::layer::{CellRef, Subplot};
use crate::line::LineStylePreset;
use crate::text::RichText;

impl Color {
    /// The same colour with alpha `a` (`0` transparent, `1` opaque).
    pub const fn with_alpha(self, a: f32) -> Color {
        Color::new(self.r, self.g, self.b, a)
    }
}

impl Annotation {
    /// A solid reference line at `value` on `axis` (vertical for `X`,
    /// horizontal for `Y`), `width` logical pixels wide, without a label.
    pub fn reference_line(axis: AnnotationAxis, value: f64, color: Color, width: f32) -> Self {
        Self::new(AnnotationKind::ReferenceLine {
            axis,
            value,
            style: AnnotationLineStyle {
                color,
                width,
                style: LineStylePreset::Solid,
            },
            label: None,
        })
    }

    /// A filled band from `from` to `to` on `axis`, across the whole plot in
    /// the other direction, drawn below the data.
    pub fn span(axis: AnnotationAxis, from: f64, to: f64, fill: Color) -> Self {
        Self::new(AnnotationKind::Span {
            axis,
            from,
            to,
            fill,
        })
    }

    /// A plain text block in `color` at `size` logical pixels, its top-left
    /// corner at `anchor`, without a box.
    pub fn text(anchor: Anchor, text: &str, color: Color, size: f32) -> Self {
        Self::text_rich(anchor, RichText::plain(text, color, size, ""))
    }

    /// A text block of styled or TeX text, its top-left corner at `anchor`,
    /// without a box.
    pub fn text_rich(anchor: Anchor, text: RichText) -> Self {
        Self::new(AnnotationKind::Text {
            anchor,
            text,
            box_style: None,
            align: crate::defaults::note_align(),
        })
    }

    /// The item with `label` at a reference line's end or beside an arrow's
    /// shaft; other kinds are returned unchanged.
    pub fn with_label(mut self, label: RichText) -> Self {
        match &mut self.kind {
            AnnotationKind::ReferenceLine { label: slot, .. }
            | AnnotationKind::Arrow { text: slot, .. } => *slot = Some(label),
            _ => {}
        }
        self
    }

    /// The item with its line or border drawn in `style` (dashed, dotted,
    /// ...); a span and a text block are returned unchanged.
    pub fn with_line_style(mut self, style: LineStylePreset) -> Self {
        match &mut self.kind {
            AnnotationKind::ReferenceLine { style: line, .. } => line.style = style,
            AnnotationKind::Arrow { style: arrow, .. } => arrow.line.style = style,
            AnnotationKind::Rect { style: shape, .. }
            | AnnotationKind::Ellipse { style: shape, .. }
            | AnnotationKind::Polygon { style: shape, .. } => shape.border.style = style,
            AnnotationKind::Span { .. } | AnnotationKind::Text { .. } => {}
        }
        self
    }

    /// The text block framed and padded by `box_style`; other kinds are
    /// returned unchanged.
    pub fn with_box(mut self, box_style: BoxStyle) -> Self {
        if let AnnotationKind::Text {
            box_style: slot, ..
        } = &mut self.kind
        {
            *slot = Some(box_style);
        }
        self
    }

    /// The text block with its point `align` on the anchor; other kinds are
    /// returned unchanged.
    pub fn with_align(mut self, align: NoteAlign) -> Self {
        if let AnnotationKind::Text { align: slot, .. } = &mut self.kind {
            *slot = align;
        }
        self
    }
}

impl AxisOptions {
    /// The right-hand y axis of an overlay layer: the default y axis with
    /// `title` shown and the title and tick labels in `color`, the colour
    /// of the series it scales.
    pub fn overlay_y(title: &str, color: Color) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_y();
        axis.title_option.text.segments = crate::text::rich_segments_from_text(title);
        axis.title_option.text.color = color;
        axis.title_option.visible = true;
        axis.label_style.color = color;
        axis
    }
}

impl Subplot {
    /// A subplot drawing `config` with the series `series` names, its axes
    /// its own and no inset marker.
    pub fn new(config: Config, series: &[&str]) -> Self {
        let mut subplot = crate::defaults::subplot();
        subplot.config = config;
        subplot.series = series.iter().map(|id| (*id).to_owned()).collect();
        subplot
    }

    /// The subplot with its x axis following `target`'s `bottom_x` range,
    /// scale and ticks.
    pub fn with_link_x(mut self, target: CellRef) -> Self {
        self.link_x = Some(target);
        self
    }

    /// The subplot with its y axis following `target`'s `left_y` range,
    /// scale and ticks.
    pub fn with_link_y(mut self, target: CellRef) -> Self {
        self.link_y = Some(target);
        self
    }
}

#[cfg(test)]
mod tests;
