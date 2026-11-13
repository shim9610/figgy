//! Constructors for the parts a host adds to a chart (#265): a colour with
//! another alpha, annotation items, overlay axes (a right-hand Y axis and a
//! top X axis), an axis unit (#273) and a subplot.
//!
//! Each builds the part's documented default and then assigns the fields it
//! names, so a field added to a type later takes its default here too.

use crate::annotation::{
    Anchor, Annotation, AnnotationAxis, AnnotationKind, AnnotationLineStyle, BoxStyle, NoteAlign,
};
use crate::color::Color;
use crate::config::{AxisOptions, AxisScale, AxisUnit, Config};
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

    /// A text block in `color` at `size` logical pixels, its top-left
    /// corner at `anchor`, without a box. `$…$`, `\(…\)` and `$$…$$` spans
    /// are TeX and `\$` is a dollar sign
    /// ([`crate::text::rich_segments_from_label`]).
    pub fn text(anchor: Anchor, text: &str, color: Color, size: f32) -> Self {
        Self::text_rich(anchor, RichText::from_label(text, color, size, ""))
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
    /// of the series it scales. `$…$` spans in `title` are TeX
    /// ([`crate::text::rich_segments_from_label`]).
    pub fn overlay_y(title: &str, color: Color) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_y();
        axis.title_option.text.segments = crate::text::rich_segments_from_label(title);
        axis.title_option.text.color = color;
        axis.title_option.visible = true;
        axis.label_style.color = color;
        axis
    }

    /// The top x axis of an overlay layer with an X axis of its own
    /// (`OverlayLayer::link_x` off): the default x axis with `title` shown
    /// and the title and tick labels in `color`, the colour of the series it
    /// scales. `$…$` spans in `title` are TeX
    /// ([`crate::text::rich_segments_from_label`]).
    pub fn overlay_x(title: &str, color: Color) -> AxisOptions {
        let mut axis = crate::defaults::axis_options_x();
        axis.title_option.text.segments = crate::text::rich_segments_from_label(title);
        axis.title_option.text.color = color;
        axis.title_option.visible = true;
        axis.label_style.color = color;
        axis
    }

    /// The axis printing its values as `scale * value + offset` (#273):
    /// raw counts as volts (`unit(5.0 / 32768.0, 0.0)`), kelvin as degrees
    /// Celsius (`unit(1.0, -273.15)`). The data, `min` and `max` stay in
    /// data units ([`AxisOptions::unit`]).
    ///
    /// `major_spacing` is read in shown units, so on a linear or
    /// logarithmic axis that takes the unit it is chosen again for the
    /// shown range, as `Chart::set_x_range` chooses it; set it after this
    /// call to keep another one. A unit the axis refuses
    /// ([`AxisOptions::validate_unit`]) is stored as given, and the chart
    /// that holds it is refused.
    pub fn unit(mut self, scale: f64, offset: f64) -> AxisOptions {
        self.unit = Some(AxisUnit::new(scale, offset));
        if self.effective_unit().is_some()
            && matches!(self.scale, AxisScale::Linear | AxisScale::Logarithmic)
        {
            let (min, max) = self.shown_range();
            self.major_spacing = crate::tick::axis_major_spacing(&self.scale, min, max);
        }
        self
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
