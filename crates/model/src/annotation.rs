//! Annotation items and their chart-family-agnostic interaction policy.
//!
//! An [`Annotation`] is a reference line, a shaded span, a text box or an
//! arrow. Its position is stored as axis values or as [`Anchor`]s; the only
//! contact point with a chart family is an [`AnnotationSpace`], the anchor ↔
//! pixel conversion that each renderer derives from its own layout. The
//! renderer builds a [`PixelSpace`] per frame, measures text with its own
//! glyph stack and registers one [`AnnotationElement`] per drawn item in a
//! `HitMap<AnnotationLayer>`. Selection, dragging and the per-item handles
//! then ride the shared `Selectable` / `Draggable` / `Resizable` contracts,
//! exactly as the radial chart's decorations do through `DecorationLayout`.
//!
//! Drags write anchors back through the same space, so the stored document
//! keeps data coordinates where the user placed them: a data-anchored item
//! follows zoom and range changes, a ratio-anchored item stays put.

use crate::color::Color;
use crate::decoration::DecorationId;
use crate::drag::{DragConfig, Draggable};
use crate::layer::AnnotationLayer;
use crate::layout::{NudgeReject, NudgeResult, RectF, span_fraction};
use crate::line::LineStylePreset;
use crate::resize::{HANDLE_SIZE, Resizable, ResizeHandle};
use crate::select::{SELECTION_PADDING, Selectable};
use crate::sink::{Border, DrawnPart, Parts, PlacedText, Sink, Slot, StrokeStyle, TextRef, union};
use crate::text::{MeasureText, RichText, TextExtents};

/// The most items one annotation layer accepts.
pub const MAX_ANNOTATIONS: usize = 1024;

/// A document axis. `X` is horizontal on screen and `Y` vertical in every
/// chart family: rectangular Cartesian charts map them to `bottom_x` /
/// `left_y` (on a chart of category bars or boxes the categorical axis maps
/// category indices), and pie, donut and polar charts (radar charts
/// included) have neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnnotationAxis {
    X,
    Y,
}

/// Where a text box or an arrow endpoint sits.
///
/// `Data` is a coordinate on the chart's value axes (a category index on the
/// category axis of a bar or box chart); it follows zoom and range changes.
/// `Ratio` is a fraction of the plot area, `(0, 0)` at its top-left corner
/// and `(1, 1)` at its bottom-right corner, and stays fixed on screen. Values
/// outside `0..1` are allowed and place the item outside the plot area but
/// inside the panel.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Anchor {
    Data { x: f64, y: f64 },
    Ratio { x: f32, y: f32 },
}

/// The kind of an [`Anchor`], for conversions that keep the screen position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnchorKind {
    Data,
    Ratio,
}

impl Anchor {
    pub fn kind(&self) -> AnchorKind {
        match self {
            Self::Data { .. } => AnchorKind::Data,
            Self::Ratio { .. } => AnchorKind::Ratio,
        }
    }

    /// The same screen position expressed in another anchor kind. `None`
    /// when the anchor cannot be placed in `space` (a data anchor on a chart
    /// without value axes, a non-finite coordinate, a non-positive value on
    /// a logarithmic axis).
    pub fn converted(&self, space: &dyn AnnotationSpace, kind: AnchorKind) -> Option<Anchor> {
        if self.kind() == kind {
            return Some(*self);
        }
        space.px_to_anchor(space.anchor_to_px(self)?, kind)
    }
}

/// Stroke of a reference line or an arrow shaft, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct AnnotationLineStyle {
    pub color: Color,
    pub width: f32,
    pub style: LineStylePreset,
}

impl Default for AnnotationLineStyle {
    fn default() -> Self {
        Self {
            color: Color::from_rgb8(60, 60, 60),
            width: 1.5,
            style: LineStylePreset::Solid,
        }
    }
}

/// The frame drawn behind a text box: fill, border and the padding between
/// the border and the text, in logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct BoxStyle {
    pub fill: Color,
    pub border_color: Color,
    pub border_width: f32,
    pub padding: f32,
}

impl Default for BoxStyle {
    fn default() -> Self {
        Self {
            fill: Color::from_rgba8(255, 255, 255, 230),
            border_color: Color::from_rgb8(60, 60, 60),
            border_width: 1.0,
            padding: 4.0,
        }
    }
}

/// Shaft stroke plus the filled triangular head at the arrow's `to` end, in
/// logical pixels.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct ArrowStyle {
    pub line: AnnotationLineStyle,
    pub head_length: f32,
    pub head_width: f32,
}

impl Default for ArrowStyle {
    fn default() -> Self {
        Self {
            line: AnnotationLineStyle::default(),
            head_length: 10.0,
            head_width: 8.0,
        }
    }
}

/// The chords an ellipse annotation is drawn with, in the raster and the
/// vector export alike: a geometry constant, not a data count.
pub const ELLIPSE_SEGMENTS: usize = 64;

/// The fill and border of a shape annotation (a rectangle, an ellipse, a
/// polygon), the border in logical pixels.
#[derive(Clone, Debug, Default, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct ShapeStyle {
    /// The fill; `None` draws the outline only.
    pub fill: Option<Color>,
    /// The border stroke; a zero width or a transparent colour draws none.
    pub border: AnnotationLineStyle,
    /// Draw the shape below the data (the underlay, as a span is) instead
    /// of above every decoration.
    pub below: bool,
}

/// The geometry and content of one annotation.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AnnotationKind {
    /// A line across the whole plot area at `value` on `axis`: a vertical
    /// line for `X`, a horizontal line for `Y`. Drawn on the overlay. The
    /// optional label sits at the line's top (`X`) or left (`Y`) end.
    ReferenceLine {
        axis: AnnotationAxis,
        value: f64,
        style: AnnotationLineStyle,
        label: Option<RichText>,
    },
    /// A filled band between `from` and `to` on `axis`, spanning the whole
    /// plot area in the other direction. Drawn on the underlay, below the
    /// data, so a translucent fill never tints it.
    Span {
        axis: AnnotationAxis,
        from: f64,
        to: f64,
        fill: Color,
    },
    /// A text block whose top-left corner sits at `anchor`. With a box
    /// style the block is padded and framed; the anchor is the frame's
    /// top-left corner. Drawn on the overlay, anywhere in the panel.
    Text {
        anchor: Anchor,
        text: RichText,
        box_style: Option<BoxStyle>,
    },
    /// A shaft from `from` to `to` with a head at `to`, and an optional
    /// label centred beside the shaft's midpoint. Drawn on the overlay.
    Arrow {
        from: Anchor,
        to: Anchor,
        style: ArrowStyle,
        text: Option<RichText>,
    },
    /// The axis-aligned rectangle between two anchors, clipped to the plot
    /// area. Drawn on the overlay, or below the data with `style.below`.
    Rect {
        from: Anchor,
        to: Anchor,
        #[cfg_attr(feature = "serde", serde(default))]
        style: ShapeStyle,
    },
    /// The ellipse inscribed in the rectangle between two anchors, drawn
    /// as [`ELLIPSE_SEGMENTS`] chords and clipped to the plot area.
    Ellipse {
        from: Anchor,
        to: Anchor,
        #[cfg_attr(feature = "serde", serde(default))]
        style: ShapeStyle,
    },
    /// A closed polygon through at least three anchors, clipped to the
    /// plot area.
    Polygon {
        vertices: Vec<Anchor>,
        #[cfg_attr(feature = "serde", serde(default))]
        style: ShapeStyle,
    },
}

impl AnnotationKind {
    /// The style of a shape kind, `None` for the other kinds.
    pub fn shape_style(&self) -> Option<&ShapeStyle> {
        match self {
            Self::Rect { style, .. }
            | Self::Ellipse { style, .. }
            | Self::Polygon { style, .. } => Some(style),
            _ => None,
        }
    }
}

/// One item of the annotation layer. Addressed as
/// `DecorationId::Annotation(index)` by its position in
/// [`AnnotationLayer::items`]; `id` is a free host label that survives
/// reordering.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Annotation {
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "String::is_empty")
    )]
    pub id: String,
    /// A hidden item is neither drawn nor registered in the hit map; its
    /// geometry is kept.
    #[cfg_attr(
        feature = "serde",
        serde(default = "default_visible", skip_serializing_if = "is_visible")
    )]
    pub visible: bool,
    pub kind: AnnotationKind,
}

#[cfg(feature = "serde")]
fn default_visible() -> bool {
    true
}

#[cfg(feature = "serde")]
fn is_visible(visible: &bool) -> bool {
    *visible
}

impl Annotation {
    pub fn new(kind: AnnotationKind) -> Self {
        Self {
            id: String::new(),
            visible: true,
            kind,
        }
    }

    /// Whether this item is drawn below the data (`Span`, a shape with
    /// `style.below`) or above every decoration (everything else).
    pub fn is_underlay(&self) -> bool {
        matches!(self.kind, AnnotationKind::Span { .. })
            || self.kind.shape_style().is_some_and(|s| s.below)
    }

    fn texts(&self) -> impl Iterator<Item = &RichText> {
        let (a, b) = match &self.kind {
            AnnotationKind::ReferenceLine { label, .. } => (label.as_ref(), None),
            AnnotationKind::Span { .. }
            | AnnotationKind::Rect { .. }
            | AnnotationKind::Ellipse { .. }
            | AnnotationKind::Polygon { .. } => (None, None),
            AnnotationKind::Text { text, .. } => (Some(text), None),
            AnnotationKind::Arrow { text, .. } => (text.as_ref(), None),
        };
        a.into_iter().chain(b)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.id.chars().count() > 160 || self.id.chars().any(char::is_control) {
            return Err("annotation id must be at most 160 characters without control characters");
        }
        for text in self.texts() {
            validate_rich_text(text)?;
        }
        match &self.kind {
            AnnotationKind::ReferenceLine { style, .. } => style.validate(),
            AnnotationKind::Span { fill, .. } => validate_color(*fill),
            AnnotationKind::Text { box_style, .. } => match box_style {
                Some(b) => {
                    validate_color(b.fill)?;
                    validate_color(b.border_color)?;
                    if !range(b.border_width, 0., 64.) || !range(b.padding, 0., 64.) {
                        return Err("annotation box border width and padding must be 0..64");
                    }
                    Ok(())
                }
                None => Ok(()),
            },
            AnnotationKind::Arrow { style, .. } => {
                style.line.validate()?;
                if !range(style.head_length, 0., 256.) || !range(style.head_width, 0., 256.) {
                    return Err("annotation arrow head must be 0..256 logical pixels");
                }
                Ok(())
            }
            AnnotationKind::Rect { style, .. } | AnnotationKind::Ellipse { style, .. } => {
                style.validate()
            }
            AnnotationKind::Polygon { vertices, style } => {
                if vertices.len() < 3 {
                    return Err("annotation polygon needs at least three vertices");
                }
                style.validate()
            }
        }
    }

    /// Multiply every pixel-based dimension by `s`; anchors and values are
    /// untouched. Used by `Config::scale_in_place` for high-DPI export.
    pub fn scale_in_place(&mut self, s: f32) {
        match &mut self.kind {
            AnnotationKind::ReferenceLine { style, label, .. } => {
                style.width *= s;
                if let Some(label) = label {
                    scale_rich_text(label, s);
                }
            }
            AnnotationKind::Span { .. } => {}
            AnnotationKind::Text {
                text, box_style, ..
            } => {
                scale_rich_text(text, s);
                if let Some(b) = box_style {
                    b.border_width *= s;
                    b.padding *= s;
                }
            }
            AnnotationKind::Arrow { style, text, .. } => {
                style.line.width *= s;
                style.head_length *= s;
                style.head_width *= s;
                if let Some(text) = text {
                    scale_rich_text(text, s);
                }
            }
            AnnotationKind::Rect { style, .. }
            | AnnotationKind::Ellipse { style, .. }
            | AnnotationKind::Polygon { style, .. } => style.border.width *= s,
        }
    }
}

impl ShapeStyle {
    fn validate(&self) -> Result<(), &'static str> {
        self.border.validate()?;
        match self.fill {
            Some(fill) => validate_color(fill),
            None => Ok(()),
        }
    }
}

fn scale_rich_text(text: &mut RichText, s: f32) {
    text.font_size *= s;
    for segment in &mut text.segments {
        if let Some(size) = &mut segment.font_size {
            *size *= s;
        }
    }
}

fn range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}

fn validate_color(c: Color) -> Result<(), &'static str> {
    if [c.r, c.g, c.b, c.a].into_iter().all(|v| range(v, 0., 1.)) {
        Ok(())
    } else {
        Err("annotation colors must be finite 0..1 RGBA")
    }
}

impl AnnotationLineStyle {
    fn validate(&self) -> Result<(), &'static str> {
        validate_color(self.color)?;
        if !range(self.width, 0., 64.) {
            return Err("annotation line width must be 0..64 logical pixels");
        }
        Ok(())
    }
}

/// The rich text limits of [`RichText::within_limits`], shared with chart
/// titles.
fn validate_rich_text(t: &RichText) -> Result<(), &'static str> {
    t.validate_math().map_err(|e| e.as_str())?;
    if !t.within_limits() {
        return Err(
            "invalid annotation text: size 6..160, at most 512 characters / 8 lines, finite styles",
        );
    }
    Ok(())
}

impl AnnotationLayer {
    /// Reject malformed styles and texts. Non-finite values and anchors are
    /// accepted: the renderer reports them as warnings and skips the item,
    /// so a document keeps serializing while a host edits it.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.items.len() > MAX_ANNOTATIONS {
            return Err("too many annotations");
        }
        self.items.iter().try_for_each(Annotation::validate)
    }

    /// Multiply every pixel-based dimension of every item by `s`.
    pub fn scale_in_place(&mut self, s: f32) {
        for item in &mut self.items {
            item.scale_in_place(s);
        }
    }

    /// The index named by a decoration selection, when it is an annotation.
    pub fn selected_index(selected: Option<&DecorationId>) -> Option<usize> {
        match selected {
            Some(DecorationId::Annotation(index)) => Some(*index),
            _ => None,
        }
    }
}

// Anchor space.

/// How one document axis maps values to a fraction of the plot area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisMap {
    /// A linear value axis over `min..max`.
    Linear { min: f64, max: f64 },
    /// A logarithmic value axis over `min..max` (both positive). Non-positive
    /// values have no position, as on line and scatter series.
    Log { min: f64, max: f64 },
    /// The category axis of a bar or box chart: category `i` is centred at
    /// coordinate `i`, so `-0.5..0.5` is the first category's slot.
    Index { count: usize },
    /// No value axis (pie, donut, radar and polar charts): data anchors,
    /// reference lines and spans have no position, so only ratio anchors can
    /// be placed.
    None,
}

/// One axis of a [`PixelSpace`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisSpace {
    pub map: AxisMap,
    /// Whether the axis runs against the screen convention: for `X` from
    /// right to left, for `Y` from top to bottom. A horizontal bar chart's
    /// category axis is inverted because its first category is at the top.
    pub inverted: bool,
}

impl AxisSpace {
    pub const NONE: Self = Self {
        map: AxisMap::None,
        inverted: false,
    };

    /// Where `value` falls along the axis, as a fraction measured from the
    /// axis' visual start (left or bottom). Not clamped.
    pub fn fraction(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let t = match self.map {
            AxisMap::Linear { min, max } => span_fraction(value, min, max, false)?,
            AxisMap::Log { min, max } => span_fraction(value, min, max, true)?,
            AxisMap::Index { count } => {
                if count == 0 {
                    return None;
                }
                (value + 0.5) / count as f64
            }
            AxisMap::None => return None,
        };
        Some(if self.inverted { 1.0 - t } else { t })
    }

    /// The value at `fraction` of the axis, the inverse of [`Self::fraction`].
    pub fn value(&self, fraction: f64) -> Option<f64> {
        if !fraction.is_finite() {
            return None;
        }
        let t = if self.inverted {
            1.0 - fraction
        } else {
            fraction
        };
        let v = match self.map {
            AxisMap::Linear { min, max } => min + t * (max - min),
            AxisMap::Log { min, max } => {
                if min <= 0.0 || max <= 0.0 {
                    return None;
                }
                10f64.powf(min.log10() + t * (max.log10() - min.log10()))
            }
            AxisMap::Index { count } => {
                if count == 0 {
                    return None;
                }
                t * count as f64 - 0.5
            }
            AxisMap::None => return None,
        };
        v.is_finite().then_some(v)
    }

    pub fn has_values(&self) -> bool {
        !matches!(self.map, AxisMap::None)
    }
}

/// The anchor ↔ pixel conversion of one chart frame: the only thing the
/// annotation layer needs from a chart family. Pixels are logical,
/// chart-local for the radial chart and in the chart area's frame for
/// Cartesian charts, so hit maps and drag deltas use the same units as the
/// rest of the interaction contracts.
pub trait AnnotationSpace {
    /// The data area; reference lines and spans are clipped to it and ratio
    /// anchors are fractions of it.
    fn plot_rect(&self) -> RectF;
    /// The whole chart panel; text boxes and arrows may sit anywhere in it.
    fn panel_rect(&self) -> RectF;
    /// Whether the `X` and `Y` axes carry values (data anchors are placeable).
    fn value_axes(&self) -> [bool; 2];
    /// Where `value` on `axis` falls, as a screen coordinate along that axis.
    fn value_to_px(&self, axis: AnnotationAxis, value: f64) -> Option<f32>;
    /// The value on `axis` at screen coordinate `px`.
    fn px_to_value(&self, axis: AnnotationAxis, px: f32) -> Option<f64>;
    fn anchor_to_px(&self, anchor: &Anchor) -> Option<[f32; 2]>;
    fn px_to_anchor(&self, px: [f32; 2], kind: AnchorKind) -> Option<Anchor>;
}

/// The concrete [`AnnotationSpace`] every renderer family produces from its
/// layout; small enough to be copied into each hit-map element so drags can
/// convert deltas without the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelSpace {
    pub panel: RectF,
    pub plot: RectF,
    pub x: AxisSpace,
    pub y: AxisSpace,
}

impl PixelSpace {
    fn axis(&self, axis: AnnotationAxis) -> &AxisSpace {
        match axis {
            AnnotationAxis::X => &self.x,
            AnnotationAxis::Y => &self.y,
        }
    }

    fn fraction_to_px(&self, axis: AnnotationAxis, t: f64) -> f32 {
        match axis {
            AnnotationAxis::X => self.plot.x + t as f32 * self.plot.width,
            AnnotationAxis::Y => self.plot.y + self.plot.height - t as f32 * self.plot.height,
        }
    }

    fn px_to_fraction(&self, axis: AnnotationAxis, px: f32) -> Option<f64> {
        if !px.is_finite() {
            return None;
        }
        Some(match axis {
            AnnotationAxis::X => {
                if self.plot.width <= 0.0 {
                    return None;
                }
                f64::from((px - self.plot.x) / self.plot.width)
            }
            AnnotationAxis::Y => {
                if self.plot.height <= 0.0 {
                    return None;
                }
                f64::from((self.plot.y + self.plot.height - px) / self.plot.height)
            }
        })
    }

    /// Where `value` falls along `axis`, as a fraction of the plot measured
    /// from the left or bottom edge; `None` when it has no position.
    pub fn fraction(&self, axis: AnnotationAxis, value: f64) -> Option<f64> {
        self.axis(axis).fraction(value)
    }
}

impl AnnotationSpace for PixelSpace {
    fn plot_rect(&self) -> RectF {
        self.plot
    }

    fn panel_rect(&self) -> RectF {
        self.panel
    }

    fn value_axes(&self) -> [bool; 2] {
        [self.x.has_values(), self.y.has_values()]
    }

    fn value_to_px(&self, axis: AnnotationAxis, value: f64) -> Option<f32> {
        let t = self.axis(axis).fraction(value)?;
        let px = self.fraction_to_px(axis, t);
        px.is_finite().then_some(px)
    }

    fn px_to_value(&self, axis: AnnotationAxis, px: f32) -> Option<f64> {
        self.axis(axis).value(self.px_to_fraction(axis, px)?)
    }

    fn anchor_to_px(&self, anchor: &Anchor) -> Option<[f32; 2]> {
        let px = match *anchor {
            Anchor::Data { x, y } => [
                self.value_to_px(AnnotationAxis::X, x)?,
                self.value_to_px(AnnotationAxis::Y, y)?,
            ],
            Anchor::Ratio { x, y } => [
                self.plot.x + x * self.plot.width,
                self.plot.y + y * self.plot.height,
            ],
        };
        px.iter().all(|v| v.is_finite()).then_some(px)
    }

    fn px_to_anchor(&self, px: [f32; 2], kind: AnchorKind) -> Option<Anchor> {
        if px.iter().any(|v| !v.is_finite()) {
            return None;
        }
        Some(match kind {
            AnchorKind::Data => Anchor::Data {
                x: self.px_to_value(AnnotationAxis::X, px[0])?,
                y: self.px_to_value(AnnotationAxis::Y, px[1])?,
            },
            AnchorKind::Ratio => {
                if self.plot.width <= 0.0 || self.plot.height <= 0.0 {
                    return None;
                }
                Anchor::Ratio {
                    x: (px[0] - self.plot.x) / self.plot.width,
                    y: (px[1] - self.plot.y) / self.plot.height,
                }
            }
        })
    }
}

// Interaction.

/// The part of an annotation a handle drags. `Start` and `End` are a span's
/// edges in screen order (left/top and right/bottom), whichever of `from`
/// and `to` currently sits there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AnnotationHandle {
    /// A span's edge at the smaller screen coordinate.
    Start,
    /// A span's edge at the larger screen coordinate.
    End,
    /// A reference line's value.
    Value,
    /// An arrow's `from` endpoint.
    From,
    /// An arrow's `to` endpoint.
    To,
    /// A rectangle's or an ellipse's box edge or corner, by its compass
    /// handle: `W` / `E` the side at the smaller / larger screen x, `N` /
    /// `S` the same in y, a corner both.
    Box(ResizeHandle),
    /// One vertex of a polygon, by its index.
    Vertex(usize),
}

/// The renderer-measured text block of an annotation item, in logical
/// pixels: its size including box padding, the first line's ascent, the
/// padding and the font size the label gaps are relative to. The renderer
/// measures once when it registers the element, so hit bounds and the raster
/// use the same metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeasuredText {
    /// `[width, height]` of the block, padding included.
    pub size: [f32; 2],
    /// The first line's rise above its baseline.
    pub ascent: f32,
    /// The box padding on every side (zero without a box).
    pub padding: f32,
    /// The text's font size: the em the label gaps are measured in.
    pub em: f32,
}

impl MeasuredText {
    /// The text extents without the padding.
    fn extents(&self) -> TextExtents {
        TextExtents {
            width: (self.size[0] - self.padding * 2.0).max(0.0),
            ascent: self.ascent,
            descent: (self.size[1] - self.padding * 2.0 - self.ascent).max(0.0),
        }
    }
}

/// A registered annotation: the item index, the frame's anchor space and the
/// renderer-measured text block of the item ([`MeasuredText`]), so hit
/// bounds use the same metrics as the raster at every output scale.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationElement {
    pub index: usize,
    pub space: PixelSpace,
    /// The measured text block, `None` without text.
    pub text: Option<MeasuredText>,
    /// Set on the nudge-path copies a handle drag produces; `None` moves the
    /// whole item.
    pub handle: Option<AnnotationHandle>,
}

/// Gap between an arrow's shaft and its label, in em of the label's font.
pub const ARROW_LABEL_GAP_EM: f32 = 0.5;
/// Inset of a reference-line label from the plot edge and the line, in em
/// of the label's font.
pub const LINE_LABEL_INSET_EM: f32 = 1.0 / 3.0;

/// Screen-space geometry of one item under a layer state, shared by the
/// hit bounds and the raster so they cannot disagree.
#[derive(Clone, Debug, PartialEq)]
pub enum AnnotationGeometry {
    /// A reference line from `a` to `b` (axis-aligned, clipped to the plot).
    Line { a: [f32; 2], b: [f32; 2] },
    /// A span's filled rectangle, clipped to the plot.
    Span(RectF),
    /// A text block's rectangle (padded when boxed).
    Text(RectF),
    /// An arrow's endpoints.
    Arrow { from: [f32; 2], to: [f32; 2] },
    /// A shape's closed polygon, clipped to the plot: a rectangle's four
    /// corners, an ellipse's chords, a polygon's vertices, in drawing
    /// order (one list per rasterized shape).
    Polygon(Vec<[f32; 2]>),
}

impl AnnotationElement {
    pub fn id(&self) -> DecorationId {
        DecorationId::Annotation(self.index)
    }

    fn with_handle(&self, handle: AnnotationHandle) -> Self {
        Self {
            handle: Some(handle),
            ..self.clone()
        }
    }

    /// `[width, height]` of the text block, `None` without text.
    pub fn text_size(&self) -> Option<[f32; 2]> {
        self.text.map(|t| t.size)
    }

    /// Where the item of `cfg` at this element's index is drawn, or `None`
    /// when it is hidden, absent, outside the plot or unplaceable.
    pub fn geometry(&self, cfg: &AnnotationLayer) -> Option<AnnotationGeometry> {
        let item = cfg.items.get(self.index)?;
        if !item.visible {
            return None;
        }
        geometry(&item.kind, &self.space, self.text_size())
    }

    /// The parts of this item that are its own, not its label: the union of
    /// everything but the text of a line or an arrow. An item that paints
    /// only its label keeps the label's box.
    fn own_extent(&self, cfg: &AnnotationLayer) -> Option<RectF> {
        let mut own: Option<RectF> = None;
        let mut all: Option<RectF> = None;
        let text_only = matches!(
            cfg.items.get(self.index).map(|item| &item.kind),
            Some(AnnotationKind::Text { .. })
        );
        AnnotationElement::emit(
            self,
            cfg,
            &mut Parts(&mut |part| {
                let r = part.extent();
                all = Some(all.map_or(r, |u| union(u, r)));
                if text_only || !matches!(part, DrawnPart::Text(_)) {
                    own = Some(own.map_or(r, |u| union(u, r)));
                }
            }),
        );
        own.or(all)
    }

    /// The handles this item exposes, as compass names on its selection box
    /// and the part each one drags, with the square's centre.
    fn handles(
        &self,
        cfg: &AnnotationLayer,
        mut each: impl FnMut(ResizeHandle, AnnotationHandle, [f32; 2]),
    ) {
        let Some(geometry) = self.geometry(cfg) else {
            return;
        };
        // The handles sit on the selection box around the item's own
        // shapes, not on its label.
        let Some(rect) = self.own_extent(cfg).map(|r| r.expanded(SELECTION_PADDING)) else {
            return;
        };
        let (x0, y0) = (rect.x, rect.y);
        let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
        let (cx, cy) = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
        match geometry {
            AnnotationGeometry::Line { a, b } => {
                if a[0] == b[0] {
                    each(ResizeHandle::W, AnnotationHandle::Value, [x0, cy]);
                    each(ResizeHandle::E, AnnotationHandle::Value, [x1, cy]);
                } else {
                    each(ResizeHandle::N, AnnotationHandle::Value, [cx, y0]);
                    each(ResizeHandle::S, AnnotationHandle::Value, [cx, y1]);
                }
            }
            AnnotationGeometry::Span(_) => {
                let Some(Annotation {
                    kind: AnnotationKind::Span { axis, .. },
                    ..
                }) = cfg.items.get(self.index)
                else {
                    return;
                };
                match axis {
                    AnnotationAxis::X => {
                        each(ResizeHandle::W, AnnotationHandle::Start, [x0, cy]);
                        each(ResizeHandle::E, AnnotationHandle::End, [x1, cy]);
                    }
                    AnnotationAxis::Y => {
                        each(ResizeHandle::N, AnnotationHandle::Start, [cx, y0]);
                        each(ResizeHandle::S, AnnotationHandle::End, [cx, y1]);
                    }
                }
            }
            AnnotationGeometry::Text(_) => {}
            AnnotationGeometry::Arrow { from, to } => {
                let (from_name, to_name) = arrow_handle_names(from, to);
                each(from_name, AnnotationHandle::From, from);
                each(to_name, AnnotationHandle::To, to);
            }
            AnnotationGeometry::Polygon(_) => {
                match cfg.items.get(self.index).map(|item| &item.kind) {
                    // A box shape takes the eight compass handles of its
                    // selection box.
                    Some(AnnotationKind::Rect { .. } | AnnotationKind::Ellipse { .. }) => {
                        use ResizeHandle::*;
                        for (name, centre) in [
                            (NW, [x0, y0]),
                            (N, [cx, y0]),
                            (NE, [x1, y0]),
                            (E, [x1, cy]),
                            (SE, [x1, y1]),
                            (S, [cx, y1]),
                            (SW, [x0, y1]),
                            (W, [x0, cy]),
                        ] {
                            each(name, AnnotationHandle::Box(name), centre);
                        }
                    }
                    // A polygon's handles are its vertices; the compass
                    // name is a placeholder the square ignores
                    // (`hit_resize_handle` never answers one).
                    Some(AnnotationKind::Polygon { vertices, .. }) => {
                        for (i, v) in vertices.iter().enumerate() {
                            if let Some(p) = self.space.anchor_to_px(v) {
                                each(ResizeHandle::NW, AnnotationHandle::Vertex(i), p);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// The polygon vertex whose handle square covers `(x, y)`, for a
    /// polygon item.
    pub fn hit_vertex(&self, cfg: &AnnotationLayer, x: f32, y: f32) -> Option<usize> {
        let mut hit = None;
        self.handles(cfg, |_, part, centre| {
            if let AnnotationHandle::Vertex(i) = part
                && handle_square(centre).contains(x, y)
            {
                hit = Some(i);
            }
        });
        hit
    }

    /// Place every part the raster paints for this item into `sink`: the
    /// one placement its bounds, hit test, selection box and pixels share.
    /// A hidden, absent or unplaceable item places nothing.
    pub fn emit(&self, cfg: &AnnotationLayer, sink: &mut dyn Sink) -> bool {
        if let (Some(item), Some(geometry)) = (cfg.items.get(self.index), self.geometry(cfg)) {
            emit_annotation(&item.kind, &geometry, self.text.as_ref(), sink);
        }
        true
    }
}

/// Compass names for an arrow's endpoints: the endpoint further left is
/// `W` and the other `E`; for a vertical arrow the upper one is `N`.
fn arrow_handle_names(from: [f32; 2], to: [f32; 2]) -> (ResizeHandle, ResizeHandle) {
    if from[0] < to[0] {
        (ResizeHandle::W, ResizeHandle::E)
    } else if from[0] > to[0] {
        (ResizeHandle::E, ResizeHandle::W)
    } else if from[1] <= to[1] {
        (ResizeHandle::N, ResizeHandle::S)
    } else {
        (ResizeHandle::S, ResizeHandle::N)
    }
}

fn handle_square(center: [f32; 2]) -> RectF {
    RectF {
        x: center[0] - HANDLE_SIZE * 0.5,
        y: center[1] - HANDLE_SIZE * 0.5,
        width: HANDLE_SIZE,
        height: HANDLE_SIZE,
    }
}

/// The label of a reference line: inset by `inset` from the line's top
/// (vertical) or left (horizontal) end, on the line's right or upper side.
pub fn line_label_rect(a: [f32; 2], b: [f32; 2], size: [f32; 2], inset: f32) -> RectF {
    if a[0] == b[0] {
        RectF {
            x: a[0] + inset,
            y: a[1].min(b[1]) + inset,
            width: size[0],
            height: size[1],
        }
    } else {
        RectF {
            x: a[0].min(b[0]) + inset,
            y: a[1] - inset - size[1],
            width: size[0],
            height: size[1],
        }
    }
}

/// The label of an arrow: centred on the shaft's midpoint, pushed off the
/// shaft towards the top of the screen by `gap`.
pub fn arrow_label_rect(from: [f32; 2], to: [f32; 2], size: [f32; 2], gap: f32) -> RectF {
    let mid = [(from[0] + to[0]) * 0.5, (from[1] + to[1]) * 0.5];
    let d = [to[0] - from[0], to[1] - from[1]];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    let mut n = if len > 0.0 {
        [-d[1] / len, d[0] / len]
    } else {
        [0.0, -1.0]
    };
    if n[1] > 0.0 || (n[1] == 0.0 && n[0] < 0.0) {
        n = [-n[0], -n[1]];
    }
    let reach = gap + size[0].abs() * 0.5 * n[0].abs() + size[1] * 0.5 * n[1].abs();
    RectF {
        x: mid[0] + n[0] * reach - size[0] * 0.5,
        y: mid[1] + n[1] * reach - size[1] * 0.5,
        width: size[0],
        height: size[1],
    }
}

fn clamp01(t: f64) -> f64 {
    t.clamp(0.0, 1.0)
}

/// Screen geometry of `kind` in `space`. Reference lines and spans are
/// clipped to the plot and vanish entirely outside it; text and arrows are
/// placed wherever their anchors fall and vanish when an anchor cannot be
/// placed or lies outside the panel.
pub fn geometry(
    kind: &AnnotationKind,
    space: &PixelSpace,
    text_size: Option<[f32; 2]>,
) -> Option<AnnotationGeometry> {
    let plot = space.plot;
    match kind {
        AnnotationKind::ReferenceLine { axis, value, .. } => {
            let t = space.fraction(*axis, *value)?;
            if !(0.0..=1.0).contains(&t) {
                return None;
            }
            let px = space.fraction_to_px(*axis, t);
            Some(match axis {
                AnnotationAxis::X => AnnotationGeometry::Line {
                    a: [px, plot.y],
                    b: [px, plot.y + plot.height],
                },
                AnnotationAxis::Y => AnnotationGeometry::Line {
                    a: [plot.x, px],
                    b: [plot.x + plot.width, px],
                },
            })
        }
        AnnotationKind::Span { axis, from, to, .. } => {
            let (ta, tb) = (space.fraction(*axis, *from)?, space.fraction(*axis, *to)?);
            let (lo, hi) = (ta.min(tb), ta.max(tb));
            if hi < 0.0 || lo > 1.0 {
                return None;
            }
            let (lo, hi) = (clamp01(lo), clamp01(hi));
            let (p0, p1) = (
                space.fraction_to_px(*axis, lo),
                space.fraction_to_px(*axis, hi),
            );
            let (p0, p1) = (p0.min(p1), p0.max(p1));
            Some(AnnotationGeometry::Span(match axis {
                AnnotationAxis::X => RectF {
                    x: p0,
                    y: plot.y,
                    width: p1 - p0,
                    height: plot.height,
                },
                AnnotationAxis::Y => RectF {
                    x: plot.x,
                    y: p0,
                    width: plot.width,
                    height: p1 - p0,
                },
            }))
        }
        AnnotationKind::Text { anchor, .. } => {
            let p = space.anchor_to_px(anchor)?;
            let size = text_size?;
            let rect = RectF {
                x: p[0],
                y: p[1],
                width: size[0],
                height: size[1],
            };
            inside_panel(space, &rect).then_some(AnnotationGeometry::Text(rect))
        }
        AnnotationKind::Arrow { from, to, .. } => {
            let (from, to) = (space.anchor_to_px(from)?, space.anchor_to_px(to)?);
            let panel = space.panel;
            (panel.contains(from[0], from[1]) && panel.contains(to[0], to[1]))
                .then_some(AnnotationGeometry::Arrow { from, to })
        }
        AnnotationKind::Rect { from, to, .. } => {
            let (a, b) = (space.anchor_to_px(from)?, space.anchor_to_px(to)?);
            let (x0, x1) = (a[0].min(b[0]), a[0].max(b[0]));
            let (y0, y1) = (a[1].min(b[1]), a[1].max(b[1]));
            shape_geometry(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], plot)
        }
        AnnotationKind::Ellipse { from, to, .. } => {
            let (a, b) = (space.anchor_to_px(from)?, space.anchor_to_px(to)?);
            let centre = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            let radii = [(a[0] - b[0]).abs() * 0.5, (a[1] - b[1]).abs() * 0.5];
            let points = (0..ELLIPSE_SEGMENTS)
                .map(|k| {
                    let angle = std::f32::consts::TAU * k as f32 / ELLIPSE_SEGMENTS as f32;
                    [
                        centre[0] + radii[0] * angle.cos(),
                        centre[1] + radii[1] * angle.sin(),
                    ]
                })
                .collect();
            shape_geometry(points, plot)
        }
        AnnotationKind::Polygon { vertices, .. } => {
            let points = vertices
                .iter()
                .map(|v| space.anchor_to_px(v))
                .collect::<Option<Vec<_>>>()?;
            shape_geometry(points, plot)
        }
    }
}

/// A shape's screen polygon clipped to the plot area, or `None` when
/// nothing with an area is left inside it.
fn shape_geometry(points: Vec<[f32; 2]>, plot: RectF) -> Option<AnnotationGeometry> {
    let clipped = clip_polygon(points, plot);
    (clipped.len() >= 3 && polygon_area(&clipped).abs() > 1e-3)
        .then_some(AnnotationGeometry::Polygon(clipped))
}

/// `points` clipped to `rect` (Sutherland–Hodgman against its four edges),
/// consecutive duplicates dropped. A polygon entirely outside is empty.
pub fn clip_polygon(points: Vec<[f32; 2]>, rect: RectF) -> Vec<[f32; 2]> {
    let (x0, y0) = (rect.x, rect.y);
    let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
    // Each edge as (inside test, intersection with the edge line).
    let edges: [ClipEdge; 4] = [
        (|p, v| p[0] >= v, cross_x, x0),
        (|p, v| p[0] <= v, cross_x, x1),
        (|p, v| p[1] >= v, cross_y, y0),
        (|p, v| p[1] <= v, cross_y, y1),
    ];
    let mut input = points;
    for (inside, cross, value) in edges {
        if input.is_empty() {
            break;
        }
        let mut output = Vec::with_capacity(input.len() + 4);
        let mut prev = input[input.len() - 1];
        for &current in &input {
            let (prev_in, current_in) = (inside(prev, value), inside(current, value));
            if current_in {
                if !prev_in {
                    output.push(cross(prev, current, value));
                }
                output.push(current);
            } else if prev_in {
                output.push(cross(prev, current, value));
            }
            prev = current;
        }
        input = output;
    }
    input.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6);
    if input.len() > 1
        && (input[0][0] - input[input.len() - 1][0]).abs() < 1e-6
        && (input[0][1] - input[input.len() - 1][1]).abs() < 1e-6
    {
        input.pop();
    }
    input
}

/// One clip edge: the inside test of a point against the edge's
/// coordinate, the crossing of a segment with the edge's line, and the
/// coordinate.
type ClipEdge = (
    fn([f32; 2], f32) -> bool,
    fn([f32; 2], [f32; 2], f32) -> [f32; 2],
    f32,
);

/// Where the segment `a`..`b` crosses the vertical line `x = v`.
fn cross_x(a: [f32; 2], b: [f32; 2], v: f32) -> [f32; 2] {
    let t = if (b[0] - a[0]).abs() > 0.0 {
        (v - a[0]) / (b[0] - a[0])
    } else {
        0.0
    };
    [v, a[1] + (b[1] - a[1]) * t]
}

/// Where the segment `a`..`b` crosses the horizontal line `y = v`.
fn cross_y(a: [f32; 2], b: [f32; 2], v: f32) -> [f32; 2] {
    let t = if (b[1] - a[1]).abs() > 0.0 {
        (v - a[1]) / (b[1] - a[1])
    } else {
        0.0
    };
    [a[0] + (b[0] - a[0]) * t, v]
}

/// The signed area of a closed polygon (the shoelace sum), positive when
/// its vertices turn clockwise on screen (y down).
pub fn polygon_area(points: &[[f32; 2]]) -> f32 {
    let mut twice = 0.0;
    for i in 0..points.len() {
        let (a, b) = (points[i], points[(i + 1) % points.len()]);
        twice += a[0] * b[1] - b[0] * a[1];
    }
    twice * 0.5
}

/// Whether `rect` lies inside the panel, with a 1 px tolerance for the
/// rounding of a drag delta.
fn inside_panel(space: &PixelSpace, rect: &RectF) -> bool {
    let p = space.panel.expanded(1.0);
    rect.x >= p.x
        && rect.y >= p.y
        && rect.x + rect.width <= p.x + p.width
        && rect.y + rect.height <= p.y + p.height
}

/// A label block placed at `rect`: its text baseline sits `ascent` below the
/// block's top.
fn placed_label<'a>(text: &'a RichText, rect: RectF, measured: &MeasuredText) -> PlacedText<'a> {
    PlacedText {
        text: TextRef::Rich(text),
        origin: (rect.x, rect.y + measured.ascent),
        extents: TextExtents {
            width: measured.size[0],
            ascent: measured.ascent,
            descent: (measured.size[1] - measured.ascent).max(0.0),
        },
        rotation: None,
    }
}

/// Place what the raster paints for `kind` at `geometry` into `sink`,
/// `text` being the measured label or text block: spans below the data,
/// everything else above. The raster draws exactly these parts, and the
/// item's bounds, hit test and selection box are computed from them, so
/// what is selectable is what is drawn. A stroke or head with no width or
/// no alpha paints nothing and is absent.
pub fn emit_annotation(
    kind: &AnnotationKind,
    geometry: &AnnotationGeometry,
    text: Option<&MeasuredText>,
    sink: &mut dyn Sink,
) {
    if let (Some(style), AnnotationGeometry::Polygon(points)) = (kind.shape_style(), geometry) {
        let slot = if style.below {
            Slot::Below
        } else {
            Slot::Above
        };
        // The fill and the border are two parts, as a text box's are: one
        // part per path the vector export writes.
        if let Some(fill) = style.fill.filter(|c| c.a > 0.0) {
            sink.polygon(slot, points, Some(fill), None);
        }
        if style.border.width > 0.0 && style.border.color.a > 0.0 {
            sink.polygon(
                slot,
                points,
                None,
                Some(&stroke_style(&style.border, "annotation_shape")),
            );
        }
        return;
    }
    match (kind, geometry) {
        (AnnotationKind::ReferenceLine { style, label, .. }, AnnotationGeometry::Line { a, b }) => {
            let (a, b) = (*a, *b);
            if style.width > 0.0 && style.color.a > 0.0 {
                sink.stroke(Slot::Above, a, b, &stroke_style(style, "annotation_line"));
            }
            if let (Some(label), Some(t)) = (label, text) {
                let rect = line_label_rect(a, b, t.size, LINE_LABEL_INSET_EM * t.em);
                sink.text(Slot::Above, &placed_label(label, rect, t));
            }
        }
        (AnnotationKind::Span { fill, .. }, AnnotationGeometry::Span(rect)) => {
            sink.rect(Slot::Below, *rect, Some(*fill), None);
        }
        (
            AnnotationKind::Text {
                text: content,
                box_style,
                ..
            },
            AnnotationGeometry::Text(rect),
        ) => {
            let rect = *rect;
            if let Some(b) = box_style {
                sink.rect(Slot::Above, rect, (b.fill.a > 0.0).then_some(b.fill), None);
                if b.border_width > 0.0 && b.border_color.a > 0.0 {
                    // The border is stroked inside the box.
                    sink.rect(
                        Slot::Above,
                        rect.expanded(-b.border_width * 0.5),
                        None,
                        Some(Border {
                            color: b.border_color,
                            width: b.border_width,
                            tag: "annotation_box",
                        }),
                    );
                }
            } else {
                sink.rect(Slot::Above, rect, None, None);
            }
            if let Some(t) = text {
                sink.text(
                    Slot::Above,
                    &PlacedText {
                        text: TextRef::Rich(content),
                        origin: (rect.x + t.padding, rect.y + t.padding + t.ascent),
                        extents: t.extents(),
                        rotation: None,
                    },
                );
            }
        }
        (
            AnnotationKind::Arrow {
                style, text: label, ..
            },
            AnnotationGeometry::Arrow { from, to },
        ) => {
            let (from, to) = (*from, *to);
            let d = [to[0] - from[0], to[1] - from[1]];
            let len = d[0].hypot(d[1]);
            let head_len = style.head_length.min(len);
            let dir = if len > 0.0 {
                [d[0] / len, d[1] / len]
            } else {
                [1.0, 0.0]
            };
            let base = [to[0] - dir[0] * head_len, to[1] - dir[1] * head_len];
            let visible = style.line.color.a > 0.0;
            if style.line.width > 0.0 && visible && len > head_len {
                sink.stroke(
                    Slot::Above,
                    from,
                    base,
                    &stroke_style(&style.line, "annotation_arrow"),
                );
            }
            if head_len > 0.0 && style.head_width > 0.0 && visible {
                let half = style.head_width * 0.5;
                let n = [-dir[1] * half, dir[0] * half];
                sink.triangle(
                    Slot::Above,
                    [
                        to,
                        [base[0] + n[0], base[1] + n[1]],
                        [base[0] - n[0], base[1] - n[1]],
                    ],
                    style.line.color,
                );
            }
            if let (Some(label), Some(t)) = (label, text) {
                let rect = arrow_label_rect(from, to, t.size, ARROW_LABEL_GAP_EM * t.em);
                sink.text(Slot::Above, &placed_label(label, rect, t));
            }
        }
        _ => {}
    }
}

fn stroke_style<'a>(style: &'a AnnotationLineStyle, tag: &'a str) -> StrokeStyle<'a> {
    StrokeStyle {
        width: style.width,
        color: style.color,
        dash: style.style.pattern(),
        tag,
    }
}

/// A measure for tests: annotation bounds never depend on glyph metrics at
/// hit time, because the renderer measured the text when it registered the
/// element.
#[cfg(test)]
struct NoMeasure;
#[cfg(test)]
impl MeasureText for NoMeasure {
    fn measure_rich(&self, _: &RichText) -> crate::text::TextExtents {
        crate::text::TextExtents {
            width: 0.0,
            ascent: 0.0,
            descent: 0.0,
        }
    }
}

impl Selectable<AnnotationLayer> for AnnotationElement {
    /// Everything the raster paints for the item ([`emit_annotation`]).
    fn emit(&self, cfg: &AnnotationLayer, _: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        AnnotationElement::emit(self, cfg, sink)
    }

    /// An item is outlined and hit on its parts: a diagonal arrow's box
    /// would cover the chart.
    /// A line, span or arrow outlines its parts (a diagonal or a long thin
    /// stroke has no useful box); a text item is one box.
    fn outlines_parts(&self, cfg: &AnnotationLayer) -> bool {
        cfg.items
            .get(self.index)
            .is_some_and(|item| !matches!(item.kind, AnnotationKind::Text { .. }))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable<AnnotationLayer>> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable<AnnotationLayer>> {
        Some(self)
    }

    fn element_id(&self) -> String {
        self.id().element_id()
    }
}

impl Draggable<AnnotationLayer> for AnnotationElement {
    fn nudge_element(&self) -> AnnotationElement {
        AnnotationElement {
            handle: None,
            ..self.clone()
        }
    }
}

impl Resizable<AnnotationLayer> for AnnotationElement {
    /// The nominal part a compass handle drags: `W` / `N` the edge in
    /// screen order at the smaller coordinate, `E` / `S` the other. Only the
    /// handles [`Resizable::resize_handles`] reports are live; `resize_by`
    /// resolves a reference line's value and an arrow's endpoints itself.
    fn resize_targets(
        &self,
        handle: ResizeHandle,
    ) -> (Option<AnnotationElement>, Option<AnnotationElement>) {
        use ResizeHandle::*;
        let part = match handle {
            W | N => AnnotationHandle::Start,
            E | S => AnnotationHandle::End,
            _ => return (None, None),
        };
        let element = self.with_handle(part);
        (
            matches!(handle, W | E).then(|| element.clone()),
            matches!(handle, N | S).then_some(element),
        )
    }

    /// Only this item's own handles: a span's two edges, a reference line's
    /// two value handles, an arrow's two endpoints, none for a text box.
    fn resize_handles(
        &self,
        cfg: &AnnotationLayer,
        _: &dyn MeasureText,
        each: &mut dyn FnMut(ResizeHandle, RectF),
    ) {
        self.handles(cfg, |name, _, center| each(name, handle_square(center)));
    }

    /// Arrow endpoints take both motion components through one nudge, so a
    /// diagonal handle drag is a single accept-or-reject decision.
    fn resize_by(
        &self,
        cfg: &mut AnnotationLayer,
        measure: &dyn MeasureText,
        handle: ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        let Some(item) = cfg.items.get(self.index) else {
            return reject;
        };
        match &item.kind {
            AnnotationKind::Arrow { .. } => {
                let Some(AnnotationGeometry::Arrow { from, to }) = self.geometry(cfg) else {
                    return reject;
                };
                let (from_name, to_name) = arrow_handle_names(from, to);
                let part = if handle == from_name {
                    AnnotationHandle::From
                } else if handle == to_name {
                    AnnotationHandle::To
                } else {
                    return reject;
                };
                cfg.nudge(self.with_handle(part), measure, dx, dy)
            }
            AnnotationKind::ReferenceLine { axis, .. } | AnnotationKind::Span { axis, .. } => {
                use ResizeHandle::*;
                let live = match axis {
                    AnnotationAxis::X => matches!(handle, W | E),
                    AnnotationAxis::Y => matches!(handle, N | S),
                };
                if !live {
                    return reject;
                }
                let part = match (&item.kind, handle) {
                    (AnnotationKind::ReferenceLine { .. }, _) => AnnotationHandle::Value,
                    (_, W | N) => AnnotationHandle::Start,
                    _ => AnnotationHandle::End,
                };
                let (dx, dy) = match axis {
                    AnnotationAxis::X => (dx, 0.0),
                    AnnotationAxis::Y => (0.0, dy),
                };
                cfg.nudge(self.with_handle(part), measure, dx, dy)
            }
            AnnotationKind::Text { .. } | AnnotationKind::Polygon { .. } => reject,
            // A box shape's handle takes both motion components through one
            // nudge, as an arrow's endpoint does.
            AnnotationKind::Rect { .. } | AnnotationKind::Ellipse { .. } => cfg.nudge(
                self.with_handle(AnnotationHandle::Box(handle)),
                measure,
                dx,
                dy,
            ),
        }
    }

    /// A polygon's handles are its vertices ([`AnnotationElement::hit_vertex`]),
    /// never a compass handle.
    fn hit_resize_handle(
        &self,
        cfg: &AnnotationLayer,
        _: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<ResizeHandle> {
        if matches!(
            cfg.items.get(self.index).map(|item| &item.kind),
            Some(AnnotationKind::Polygon { .. })
        ) {
            return None;
        }
        let mut hit = None;
        self.handles(cfg, |name, _, centre| {
            if handle_square(centre).contains(x, y) {
                hit = Some(name);
            }
        });
        hit
    }
}

impl DragConfig for AnnotationLayer {
    type Element = AnnotationElement;

    /// Move an item or one of its handles by a screen delta. Values and
    /// anchors are converted back through the element's space, keeping
    /// their anchor kind. A move is rejected atomically when the result
    /// could not be drawn ([`geometry`] places nothing: a line outside the
    /// axis range, a span entirely outside the plot, a text box or arrow
    /// endpoint outside the panel), and a dragged span edge stays inside the
    /// axis range so its handle stays on screen. The element carries its
    /// measured text, so `measure` is not read.
    fn nudge(
        &mut self,
        element: AnnotationElement,
        _measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let reject = NudgeResult::Rejected(NudgeReject::OutOfBounds);
        if !dx.is_finite() || !dy.is_finite() {
            return reject;
        }
        let space = element.space;
        let Some(item) = self.items.get(element.index) else {
            return reject;
        };
        let moved = |value: f64, axis: AnnotationAxis| -> Option<f64> {
            let px = space.value_to_px(axis, value)?;
            let d = match axis {
                AnnotationAxis::X => dx,
                AnnotationAxis::Y => dy,
            };
            space.px_to_value(axis, px + d)
        };
        let in_range = |value: f64, axis: AnnotationAxis| {
            space
                .fraction(axis, value)
                .is_some_and(|t| (0.0..=1.0).contains(&t))
        };
        let moved_anchor = |anchor: &Anchor| -> Option<Anchor> {
            let p = space.anchor_to_px(anchor)?;
            space.px_to_anchor([p[0] + dx, p[1] + dy], anchor.kind())
        };
        let next = match (&item.kind, element.handle) {
            (
                AnnotationKind::ReferenceLine {
                    axis,
                    value,
                    style,
                    label,
                },
                _,
            ) => {
                let Some(v) = moved(*value, *axis) else {
                    return reject;
                };
                AnnotationKind::ReferenceLine {
                    axis: *axis,
                    value: v,
                    style: style.clone(),
                    label: label.clone(),
                }
            }
            (
                AnnotationKind::Span {
                    axis,
                    from,
                    to,
                    fill,
                },
                handle,
            ) => {
                let (mut a, mut b) = (*from, *to);
                match handle {
                    None => {
                        let (Some(na), Some(nb)) = (moved(a, *axis), moved(b, *axis)) else {
                            return reject;
                        };
                        a = na;
                        b = nb;
                    }
                    Some(part @ (AnnotationHandle::Start | AnnotationHandle::End)) => {
                        let (Some(pa), Some(pb)) =
                            (space.value_to_px(*axis, a), space.value_to_px(*axis, b))
                        else {
                            return reject;
                        };
                        // `Start` is the edge at the smaller screen coordinate.
                        let edge_is_a = (pa <= pb) == (part == AnnotationHandle::Start);
                        let edge = if edge_is_a { a } else { b };
                        let Some(v) = moved(edge, *axis).filter(|v| in_range(*v, *axis)) else {
                            return reject;
                        };
                        if edge_is_a {
                            a = v;
                        } else {
                            b = v;
                        }
                    }
                    Some(_) => return reject,
                }
                AnnotationKind::Span {
                    axis: *axis,
                    from: a,
                    to: b,
                    fill: *fill,
                }
            }
            (
                AnnotationKind::Text {
                    anchor,
                    text,
                    box_style,
                },
                None,
            ) => {
                let Some(p) = space.anchor_to_px(anchor) else {
                    return reject;
                };
                let Some(next) = space.px_to_anchor([p[0] + dx, p[1] + dy], anchor.kind()) else {
                    return reject;
                };
                AnnotationKind::Text {
                    anchor: next,
                    text: text.clone(),
                    box_style: box_style.clone(),
                }
            }
            (
                AnnotationKind::Arrow {
                    from,
                    to,
                    style,
                    text,
                },
                handle,
            ) => {
                let (mut a, mut b) = (*from, *to);
                match handle {
                    None => {
                        let (Some(na), Some(nb)) = (moved_anchor(&a), moved_anchor(&b)) else {
                            return reject;
                        };
                        a = na;
                        b = nb;
                    }
                    Some(AnnotationHandle::From) => {
                        let Some(na) = moved_anchor(&a) else {
                            return reject;
                        };
                        a = na;
                    }
                    Some(AnnotationHandle::To) => {
                        let Some(nb) = moved_anchor(&b) else {
                            return reject;
                        };
                        b = nb;
                    }
                    Some(_) => return reject,
                }
                AnnotationKind::Arrow {
                    from: a,
                    to: b,
                    style: style.clone(),
                    text: text.clone(),
                }
            }
            (
                AnnotationKind::Rect { from, to, style }
                | AnnotationKind::Ellipse { from, to, style },
                handle,
            ) => {
                let (mut a, mut b) = (*from, *to);
                match handle {
                    None => {
                        let (Some(na), Some(nb)) = (moved_anchor(&a), moved_anchor(&b)) else {
                            return reject;
                        };
                        a = na;
                        b = nb;
                    }
                    Some(AnnotationHandle::Box(name)) => {
                        let (Some(pa), Some(pb)) = (space.anchor_to_px(&a), space.anchor_to_px(&b))
                        else {
                            return reject;
                        };
                        use ResizeHandle::*;
                        // The side the handle drags: the anchor at the
                        // smaller screen coordinate for `W` / `N`, the
                        // larger for `E` / `S`; none in the other direction
                        // for an edge-midpoint handle.
                        let x_on_a = match name {
                            NW | W | SW => Some(pa[0] <= pb[0]),
                            NE | E | SE => Some(pa[0] > pb[0]),
                            N | S => None,
                        };
                        let y_on_a = match name {
                            NW | N | NE => Some(pa[1] <= pb[1]),
                            SW | S | SE => Some(pa[1] > pb[1]),
                            W | E => None,
                        };
                        let shift = |on_a: Option<bool>, this_is_a: bool, d: f32| {
                            on_a.is_some_and(|on_a| on_a == this_is_a)
                                .then_some(d)
                                .unwrap_or(0.0)
                        };
                        // An anchor the handle does not move keeps its
                        // exact value: no round trip through the space.
                        let move_by = |anchor: &Anchor, p: [f32; 2], this_is_a: bool| {
                            let (sx, sy) =
                                (shift(x_on_a, this_is_a, dx), shift(y_on_a, this_is_a, dy));
                            if sx == 0.0 && sy == 0.0 {
                                return Some(*anchor);
                            }
                            space.px_to_anchor([p[0] + sx, p[1] + sy], anchor.kind())
                        };
                        let (Some(na), Some(nb)) = (move_by(&a, pa, true), move_by(&b, pb, false))
                        else {
                            return reject;
                        };
                        a = na;
                        b = nb;
                    }
                    Some(_) => return reject,
                }
                match &item.kind {
                    AnnotationKind::Rect { .. } => AnnotationKind::Rect {
                        from: a,
                        to: b,
                        style: style.clone(),
                    },
                    _ => AnnotationKind::Ellipse {
                        from: a,
                        to: b,
                        style: style.clone(),
                    },
                }
            }
            (AnnotationKind::Polygon { vertices, style }, handle) => {
                let mut next = vertices.clone();
                match handle {
                    None => {
                        for v in &mut next {
                            let Some(moved) = moved_anchor(v) else {
                                return reject;
                            };
                            *v = moved;
                        }
                    }
                    Some(AnnotationHandle::Vertex(i)) => {
                        let Some(v) = next.get_mut(i) else {
                            return reject;
                        };
                        let Some(moved) = moved_anchor(v) else {
                            return reject;
                        };
                        *v = moved;
                    }
                    Some(_) => return reject,
                }
                AnnotationKind::Polygon {
                    vertices: next,
                    style: style.clone(),
                }
            }
            _ => return reject,
        };
        // The moved item must still be drawn: the one placement decides.
        if geometry(&next, &space, element.text_size()).is_none() {
            return reject;
        }
        self.items[element.index].kind = next;
        NudgeResult::Moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::select::SelectionGeometry;
    use crate::text::rich_segments_from_text;

    fn space() -> PixelSpace {
        PixelSpace {
            panel: RectF {
                x: 0.,
                y: 0.,
                width: 400.,
                height: 300.,
            },
            plot: RectF {
                x: 50.,
                y: 20.,
                width: 300.,
                height: 200.,
            },
            x: AxisSpace {
                map: AxisMap::Linear { min: 0., max: 10. },
                inverted: false,
            },
            y: AxisSpace {
                map: AxisMap::Log {
                    min: 1.,
                    max: 1000.,
                },
                inverted: false,
            },
        }
    }

    fn text(s: &str) -> RichText {
        RichText {
            segments: rich_segments_from_text(s),
            color: Color::BLACK,
            font_size: 12.,
            font: String::new(),
            align: Default::default(),
        }
    }

    /// An element of item `index` whose text block, when it has one, is
    /// `text_size` with no padding, a 12 px font and its baseline at the
    /// block's bottom.
    fn element(index: usize, text_size: Option<[f32; 2]>) -> AnnotationElement {
        AnnotationElement {
            index,
            space: space(),
            text: text_size.map(|size| MeasuredText {
                size,
                ascent: size[1],
                padding: 0.,
                em: 12.,
            }),
            handle: None,
        }
    }

    #[test]
    fn space_converts_values_anchors_and_back() {
        let s = space();
        assert_eq!(s.value_to_px(AnnotationAxis::X, 5.), Some(200.));
        let y10 = s.value_to_px(AnnotationAxis::Y, 10.).unwrap();
        assert!((y10 - (220. - 200. / 3.)).abs() < 1e-3);
        assert_eq!(s.value_to_px(AnnotationAxis::Y, 0.), None);
        assert_eq!(s.value_to_px(AnnotationAxis::X, f64::NAN), None);
        let back = s.px_to_value(AnnotationAxis::Y, y10).unwrap();
        assert!((back - 10.).abs() < 1e-3);
        let data = Anchor::Data { x: 2.5, y: 100. };
        let px = s.anchor_to_px(&data).unwrap();
        assert_eq!(px[0], 125.);
        let ratio = data.converted(&s, AnchorKind::Ratio).unwrap();
        assert_eq!(s.anchor_to_px(&ratio), Some(px));
        match ratio.converted(&s, AnchorKind::Data).unwrap() {
            Anchor::Data { x, y } => {
                assert!((x - 2.5).abs() < 1e-4 && (y - 100.).abs() < 1e-2)
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            s.anchor_to_px(&Anchor::Ratio { x: 0., y: 1. }),
            Some([50., 220.])
        );
        let category = AxisSpace {
            map: AxisMap::Index { count: 4 },
            inverted: true,
        };
        assert_eq!(category.fraction(0.), Some(1. - 0.125));
        assert_eq!(category.value(1. - 0.125), Some(0.));
        let mut none = s;
        none.x = AxisSpace::NONE;
        none.y = AxisSpace::NONE;
        assert_eq!(none.value_axes(), [false, false]);
        assert_eq!(none.anchor_to_px(&data), None);
        assert!(
            none.anchor_to_px(&Anchor::Ratio { x: 0.5, y: 0.5 })
                .is_some()
        );
    }

    #[test]
    fn geometry_clips_lines_and_spans_and_hides_unplaceable_items() {
        let s = space();
        let line = AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::X,
            value: 2.,
            style: Default::default(),
            label: None,
        };
        assert_eq!(
            geometry(&line, &s, None),
            Some(AnnotationGeometry::Line {
                a: [110., 20.],
                b: [110., 220.]
            })
        );
        let outside = AnnotationKind::ReferenceLine {
            axis: AnnotationAxis::X,
            value: 12.,
            style: Default::default(),
            label: None,
        };
        assert_eq!(geometry(&outside, &s, None), None);
        let span = AnnotationKind::Span {
            axis: AnnotationAxis::Y,
            from: 0.,
            to: 10.,
            fill: Color::BLACK,
        };
        // A non-positive edge on a log axis has no position at all.
        assert_eq!(geometry(&span, &s, None), None);
        let span = AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: -5.,
            to: 5.,
            fill: Color::BLACK,
        };
        assert_eq!(
            geometry(&span, &s, None),
            Some(AnnotationGeometry::Span(RectF {
                x: 50.,
                y: 20.,
                width: 150.,
                height: 200.
            }))
        );
        let text = AnnotationKind::Text {
            anchor: Anchor::Ratio { x: 1.1, y: 0. },
            text: text("a"),
            box_style: None,
        };
        assert!(geometry(&text, &s, Some([20., 10.])).is_some());
        assert_eq!(geometry(&text, &s, Some([200., 10.])), None);
        assert_eq!(geometry(&text, &s, None), None);
        let arrow = AnnotationKind::Arrow {
            from: Anchor::Data { x: 0., y: 1. },
            to: Anchor::Data { x: 10., y: 1000. },
            style: Default::default(),
            text: None,
        };
        assert_eq!(
            geometry(&arrow, &s, None),
            Some(AnnotationGeometry::Arrow {
                from: [50., 220.],
                to: [350., 20.]
            })
        );
    }

    #[test]
    fn drags_write_values_and_anchors_back_through_the_space() {
        let mut layer = AnnotationLayer {
            visible: true,
            items: vec![
                Annotation::new(AnnotationKind::ReferenceLine {
                    axis: AnnotationAxis::X,
                    value: 2.,
                    style: Default::default(),
                    label: None,
                }),
                Annotation::new(AnnotationKind::Span {
                    axis: AnnotationAxis::X,
                    from: 6.,
                    to: 4.,
                    fill: Color::BLACK,
                }),
                Annotation::new(AnnotationKind::Text {
                    anchor: Anchor::Data { x: 5., y: 10. },
                    text: text("note"),
                    box_style: None,
                }),
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Ratio { x: 0.1, y: 0.1 },
                    to: Anchor::Ratio { x: 0.5, y: 0.5 },
                    style: Default::default(),
                    text: None,
                }),
            ],
        };
        let before = layer.clone();
        let line = element(0, None);
        assert_eq!(
            line.drag_by(&mut layer, &NoMeasure, 30., 99.),
            NudgeResult::Moved
        );
        match layer.items[0].kind {
            AnnotationKind::ReferenceLine { value, .. } => assert!((value - 3.).abs() < 1e-5),
            _ => unreachable!(),
        }
        assert!(matches!(
            line.drag_by(&mut layer, &NoMeasure, 1000., 0.),
            NudgeResult::Rejected(_)
        ));
        assert!(matches!(
            line.drag_by(&mut layer, &NoMeasure, f32::NAN, 0.),
            NudgeResult::Rejected(_)
        ));
        // A vertical line's live handles are W and E; N drives nothing.
        assert_eq!(
            line.resize_by(&mut layer, &NoMeasure, ResizeHandle::E, 30., 0.),
            NudgeResult::Moved
        );
        assert!(matches!(
            line.resize_by(&mut layer, &NoMeasure, ResizeHandle::N, 0., 30.),
            NudgeResult::Rejected(_)
        ));

        let span = element(1, None);
        assert_eq!(
            span.drag_by(&mut layer, &NoMeasure, 30., 0.),
            NudgeResult::Moved
        );
        match layer.items[1].kind {
            AnnotationKind::Span { from, to, .. } => {
                assert!((from - 7.).abs() < 1e-5 && (to - 5.).abs() < 1e-5)
            }
            _ => unreachable!(),
        }
        // `Start` is the left edge in screen order: here `to`, the smaller value.
        assert_eq!(
            span.resize_by(&mut layer, &NoMeasure, ResizeHandle::W, -30., 0.),
            NudgeResult::Moved
        );
        match layer.items[1].kind {
            AnnotationKind::Span { from, to, .. } => {
                assert!((from - 7.).abs() < 1e-5 && (to - 4.).abs() < 1e-5)
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            span.drag_by(&mut layer, &NoMeasure, 400., 0.),
            NudgeResult::Rejected(_)
        ));

        let text_box = element(2, Some([40., 14.]));
        let p0 = space()
            .anchor_to_px(&Anchor::Data { x: 5., y: 10. })
            .unwrap();
        assert_eq!(
            text_box.drag_by(&mut layer, &NoMeasure, 10., -20.),
            NudgeResult::Moved
        );
        match layer.items[2].kind {
            AnnotationKind::Text { anchor, .. } => {
                let p = space().anchor_to_px(&anchor).unwrap();
                assert!((p[0] - p0[0] - 10.).abs() < 1e-3 && (p[1] - p0[1] + 20.).abs() < 1e-3);
                assert_eq!(anchor.kind(), AnchorKind::Data);
            }
            _ => unreachable!(),
        }
        // Anywhere in the panel, but not beyond it.
        assert_eq!(
            text_box.drag_by(&mut layer, &NoMeasure, 150., 0.),
            NudgeResult::Moved
        );
        assert!(matches!(
            text_box.drag_by(&mut layer, &NoMeasure, 100., 0.),
            NudgeResult::Rejected(_)
        ));

        let arrow = element(3, None);
        assert_eq!(
            arrow.drag_by(&mut layer, &NoMeasure, 10., 10.),
            NudgeResult::Moved
        );
        assert_eq!(
            arrow.resize_by(&mut layer, &NoMeasure, ResizeHandle::E, 5., -5.),
            NudgeResult::Moved
        );
        match layer.items[3].kind {
            AnnotationKind::Arrow { from, to, .. } => {
                let s = space();
                let f = s.anchor_to_px(&from).unwrap();
                let t = s.anchor_to_px(&to).unwrap();
                assert!((f[0] - 90.).abs() < 1e-3 && (f[1] - 50.).abs() < 1e-3);
                assert!((t[0] - 215.).abs() < 1e-3 && (t[1] - 125.).abs() < 1e-3);
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            arrow.resize_by(&mut layer, &NoMeasure, ResizeHandle::W, -1000., 0.),
            NudgeResult::Rejected(_)
        ));
        assert_ne!(layer, before);
        layer.validate().unwrap();
    }

    #[test]
    fn selection_boxes_carry_only_live_handles() {
        let layer = AnnotationLayer {
            visible: true,
            items: vec![
                Annotation::new(AnnotationKind::Span {
                    axis: AnnotationAxis::Y,
                    from: 10.,
                    to: 100.,
                    fill: Color::BLACK,
                }),
                Annotation::new(AnnotationKind::Text {
                    anchor: Anchor::Ratio { x: 0.5, y: 0.5 },
                    text: text("t"),
                    box_style: None,
                }),
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Ratio { x: 0.5, y: 0.5 },
                    to: Anchor::Ratio { x: 0.2, y: 0.9 },
                    style: Default::default(),
                    text: None,
                }),
            ],
        };
        let span = element(0, None);
        let sb = span.selection_box(&layer, &NoMeasure).unwrap();
        assert_eq!(sb.handles().len(), 2);
        let top = sb.handles()[0];
        assert_eq!(
            span.hit_resize_handle(&layer, &NoMeasure, top.x + 4., top.y + 4.),
            Some(ResizeHandle::N)
        );
        assert_eq!(
            span.hit_resize_handle(&layer, &NoMeasure, sb.rect().x, sb.rect().y),
            None
        );
        let text_box = element(1, Some([10., 10.]));
        assert!(
            text_box
                .selection_box(&layer, &NoMeasure)
                .unwrap()
                .handles()
                .is_empty()
        );
        let arrow = element(2, None);
        let sb = arrow.selection_box(&layer, &NoMeasure).unwrap();
        assert_eq!(sb.handles().len(), 2);
        // `from` is to the right of `to`, so it is the E handle.
        assert_eq!(
            arrow.hit_resize_handle(&layer, &NoMeasure, 200., 120.),
            Some(ResizeHandle::E)
        );
        assert_eq!(arrow.element_id(), "annotation_2");
        let mut hidden = layer.clone();
        hidden.items[2].visible = false;
        assert!(arrow.bounds(&hidden, &NoMeasure).is_none());
    }

    #[test]
    fn validation_rejects_malformed_styles_but_not_nonfinite_values() {
        let mut layer = AnnotationLayer::default();
        layer
            .items
            .push(Annotation::new(AnnotationKind::ReferenceLine {
                axis: AnnotationAxis::Y,
                value: f64::NAN,
                style: Default::default(),
                label: Some(text("nan")),
            }));
        layer.items.push(Annotation::new(AnnotationKind::Text {
            anchor: Anchor::Data {
                x: f64::INFINITY,
                y: 0.,
            },
            text: text("inf"),
            box_style: Some(BoxStyle::default()),
        }));
        layer.validate().unwrap();
        assert!(!layer.is_default());
        let mut bad = layer.clone();
        match &mut bad.items[0].kind {
            AnnotationKind::ReferenceLine { style, .. } => style.width = -1.,
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err());
        let mut bad = layer.clone();
        match &mut bad.items[1].kind {
            AnnotationKind::Text { text, .. } => text.font_size = 2.,
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err());
        let mut scaled = layer.clone();
        scaled.scale_in_place(2.);
        match (&scaled.items[1].kind, &layer.items[1].kind) {
            (
                AnnotationKind::Text {
                    text: a,
                    box_style: Some(ba),
                    anchor: aa,
                },
                AnnotationKind::Text {
                    text: b,
                    box_style: Some(bb),
                    anchor: ab,
                },
            ) => {
                assert_eq!(a.font_size, b.font_size * 2.);
                assert_eq!(ba.padding, bb.padding * 2.);
                assert_eq!(aa, ab);
            }
            _ => unreachable!(),
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn annotations_round_trip_and_omit_defaults() {
        let item = Annotation::new(AnnotationKind::Span {
            axis: AnnotationAxis::X,
            from: 1.,
            to: 2.,
            fill: Color::BLACK,
        });
        let json = serde_json::to_string(&item).unwrap();
        assert!(
            !json.contains("\"id\"") && !json.contains("\"visible\""),
            "{json}"
        );
        let back: Annotation = serde_json::from_str(&json).unwrap();
        assert_eq!(back, item);
        let layer = AnnotationLayer {
            visible: false,
            items: vec![
                Annotation {
                    id: "mean".into(),
                    visible: false,
                    ..item.clone()
                },
                Annotation::new(AnnotationKind::Arrow {
                    from: Anchor::Data { x: 1., y: 2. },
                    to: Anchor::Ratio { x: 0.5, y: 0.5 },
                    style: Default::default(),
                    text: Some(text("peak")),
                }),
            ],
        };
        let json = serde_json::to_value(&layer).unwrap();
        assert_eq!(json["items"][0]["id"], "mean");
        assert_eq!(json["items"][0]["visible"], false);
        let back: AnnotationLayer = serde_json::from_value(json).unwrap();
        assert_eq!(back, layer);
        assert_eq!(
            serde_json::to_string(&AnnotationLayer::default()).unwrap(),
            r#"{"visible":true}"#
        );
        // A shape round-trips with its default style omitted.
        let shape = Annotation::new(AnnotationKind::Polygon {
            vertices: vec![
                Anchor::Data { x: 1., y: 2. },
                Anchor::Ratio { x: 0.5, y: 0.5 },
                Anchor::Data { x: 3., y: 4. },
            ],
            style: ShapeStyle::default(),
        });
        let json = serde_json::to_value(&shape).unwrap();
        assert_eq!(
            json["kind"]["Polygon"]["style"]["fill"],
            serde_json::Value::Null
        );
        assert_eq!(json["kind"]["Polygon"]["style"]["below"], false);
        let back: Annotation = serde_json::from_value(json).unwrap();
        assert_eq!(back, shape);
        let bare: Annotation =
            serde_json::from_str(r#"{"kind":{"Rect":{"from":{"Ratio":{"x":0.1,"y":0.1}},"to":{"Ratio":{"x":0.4,"y":0.3}}}}}"#)
                .unwrap();
        assert_eq!(
            bare.kind,
            AnnotationKind::Rect {
                from: Anchor::Ratio { x: 0.1, y: 0.1 },
                to: Anchor::Ratio { x: 0.4, y: 0.3 },
                style: ShapeStyle::default(),
            }
        );
    }

    fn ratio_near(anchor: &Anchor, x: f32, y: f32) -> bool {
        matches!(anchor, Anchor::Ratio { x: ax, y: ay } if (ax - x).abs() < 1e-5 && (ay - y).abs() < 1e-5)
    }

    fn shape(kind: AnnotationKind) -> AnnotationLayer {
        AnnotationLayer {
            visible: true,
            items: vec![Annotation::new(kind)],
        }
    }

    /// A rectangle and an ellipse place their clipped polygons, a polygon
    /// its vertices; a shape outside the plot has no geometry; a concave
    /// shape is grabbed in its arms and not in its notch; the plot clips.
    #[test]
    fn shapes_place_clip_and_grab_inside_their_polygon() {
        let s = space();
        // The plot is x 50..350, y 20..220. A rectangle from (100, 50) to
        // (200, 150) in ratio terms.
        let rect = shape(AnnotationKind::Rect {
            from: Anchor::Ratio {
                x: 1. / 6.,
                y: 0.15,
            },
            to: Anchor::Ratio { x: 0.5, y: 0.65 },
            style: ShapeStyle::default(),
        });
        let e = element(0, None);
        let Some(AnnotationGeometry::Polygon(points)) = e.geometry(&rect) else {
            panic!("a rectangle places a polygon");
        };
        assert_eq!(
            points,
            vec![[100., 50.], [200., 50.], [200., 150.], [100., 150.]]
        );
        assert!(e.contains(&rect, &NoMeasure, 150., 100.));
        assert!(!e.contains(&rect, &NoMeasure, 250., 100.));
        // The bounds reach half the default border (1.5 px) past the box.
        assert_eq!(
            e.bounds(&rect, &NoMeasure),
            Some(RectF {
                x: 99.25,
                y: 49.25,
                width: 101.5,
                height: 101.5
            })
        );
        // The ellipse in the same box: its centre and its axis ends inside,
        // the box's corner outside.
        let ellipse = shape(AnnotationKind::Ellipse {
            from: Anchor::Ratio {
                x: 1. / 6.,
                y: 0.15,
            },
            to: Anchor::Ratio { x: 0.5, y: 0.65 },
            style: ShapeStyle::default(),
        });
        let Some(AnnotationGeometry::Polygon(points)) = e.geometry(&ellipse) else {
            panic!("an ellipse places a polygon");
        };
        assert_eq!(points.len(), ELLIPSE_SEGMENTS);
        assert!(e.contains(&ellipse, &NoMeasure, 150., 100.));
        assert!(e.contains(&ellipse, &NoMeasure, 150. + 47., 100.));
        assert!(e.contains(&ellipse, &NoMeasure, 150., 100. - 47.));
        assert!(!e.contains(&ellipse, &NoMeasure, 103., 53.));
        // A concave L: the arms hit, the notch does not.
        let l = shape(AnnotationKind::Polygon {
            vertices: vec![
                Anchor::Ratio { x: 0.0, y: 0.0 },
                Anchor::Ratio { x: 0.5, y: 0.0 },
                Anchor::Ratio { x: 0.5, y: 0.25 },
                Anchor::Ratio { x: 0.25, y: 0.25 },
                Anchor::Ratio { x: 0.25, y: 0.5 },
                Anchor::Ratio { x: 0.0, y: 0.5 },
            ],
            style: ShapeStyle::default(),
        });
        assert!(e.contains(&l, &NoMeasure, 60., 30.)); // the top arm
        assert!(e.contains(&l, &NoMeasure, 60., 110.)); // the left arm
        assert!(!e.contains(&l, &NoMeasure, 180., 110.)); // the notch
        // A rectangle straddling the plot's right edge is clipped to it,
        // one entirely outside places nothing.
        let straddling = shape(AnnotationKind::Rect {
            from: Anchor::Ratio { x: 0.8, y: 0.2 },
            to: Anchor::Ratio { x: 1.3, y: 0.4 },
            style: ShapeStyle::default(),
        });
        let Some(AnnotationGeometry::Polygon(points)) = e.geometry(&straddling) else {
            panic!("the straddling rectangle places a polygon");
        };
        assert!(points.iter().all(|p| p[0] <= 350. + 1e-3));
        assert!(points.iter().any(|p| (p[0] - 350.).abs() < 1e-3));
        let outside = shape(AnnotationKind::Rect {
            from: Anchor::Ratio { x: 1.1, y: 0.2 },
            to: Anchor::Ratio { x: 1.3, y: 0.4 },
            style: ShapeStyle::default(),
        });
        assert_eq!(e.geometry(&outside), None);
        // A data anchor on a chart without value axes has no geometry.
        let mut no_axes = s;
        no_axes.x = AxisSpace {
            map: AxisMap::None,
            inverted: false,
        };
        no_axes.y = no_axes.x;
        let data = shape(AnnotationKind::Rect {
            from: Anchor::Data { x: 1., y: 10. },
            to: Anchor::Data { x: 3., y: 100. },
            style: ShapeStyle::default(),
        });
        let on_no_axes = AnnotationElement {
            space: no_axes,
            ..element(0, None)
        };
        assert_eq!(on_no_axes.geometry(&data), None);
        assert!(e.geometry(&data).is_some());
    }

    /// Whole-shape drags move every anchor by the delta through the space,
    /// a box handle moves its side only, a vertex handle one vertex; a drag
    /// that would leave nothing inside the plot is rejected unchanged.
    #[test]
    fn shape_drags_move_anchors_sides_and_vertices() {
        let e = element(0, None);
        let mut rect = shape(AnnotationKind::Rect {
            from: Anchor::Ratio {
                x: 1. / 6.,
                y: 0.15,
            },
            to: Anchor::Data { x: 5., y: 10. },
            style: ShapeStyle::default(),
        });
        let before = rect.clone();
        assert_eq!(
            e.drag_by(&mut rect, &NoMeasure, 30., 20.),
            NudgeResult::Moved
        );
        let AnnotationKind::Rect { from, to, .. } = &rect.items[0].kind else {
            unreachable!()
        };
        assert!(ratio_near(from, 0.2666667, 0.25), "{from:?}");
        let Anchor::Data { x, .. } = to else {
            panic!("the data anchor keeps its kind")
        };
        assert!((x - 6.).abs() < 1e-6, "{x}");
        // The east handle moves the side at the larger x: the data anchor.
        let mut sided = before.clone();
        assert_eq!(
            e.resize_by(&mut sided, &NoMeasure, ResizeHandle::E, 30., 20.),
            NudgeResult::Moved
        );
        let AnnotationKind::Rect { from, to, .. } = &sided.items[0].kind else {
            unreachable!()
        };
        assert_eq!(
            *from,
            Anchor::Ratio {
                x: 1. / 6.,
                y: 0.15
            }
        );
        let Anchor::Data { x, y } = to else {
            panic!("the data anchor keeps its kind")
        };
        assert!((x - 6.).abs() < 1e-6 && (y - 10.).abs() < 1e-4, "{x} {y}");
        // The north-west corner moves the ratio anchor in both axes.
        let mut cornered = before.clone();
        assert_eq!(
            e.resize_by(&mut cornered, &NoMeasure, ResizeHandle::NW, 30., 20.),
            NudgeResult::Moved
        );
        let AnnotationKind::Rect { from, to, .. } = &cornered.items[0].kind else {
            unreachable!()
        };
        assert!(ratio_near(from, 0.2666667, 0.25), "{from:?}");
        assert_eq!(*to, Anchor::Data { x: 5., y: 10. });
        // A polygon's vertex handle moves that vertex only.
        let mut poly = shape(AnnotationKind::Polygon {
            vertices: vec![
                Anchor::Ratio { x: 0.1, y: 0.1 },
                Anchor::Ratio { x: 0.5, y: 0.1 },
                Anchor::Ratio { x: 0.5, y: 0.5 },
            ],
            style: ShapeStyle::default(),
        });
        assert_eq!(e.hit_vertex(&poly, 200., 120.), Some(2));
        assert_eq!(e.hit_vertex(&poly, 150., 150.), None);
        assert_eq!(e.hit_resize_handle(&poly, &NoMeasure, 200., 120.), None);
        let moved = AnnotationElement {
            handle: Some(AnnotationHandle::Vertex(2)),
            ..e.clone()
        };
        assert_eq!(poly.nudge(moved, &NoMeasure, 30., 20.), NudgeResult::Moved);
        let AnnotationKind::Polygon { vertices, .. } = &poly.items[0].kind else {
            unreachable!()
        };
        assert_eq!(vertices[0], Anchor::Ratio { x: 0.1, y: 0.1 });
        assert!(ratio_near(&vertices[2], 0.6, 0.6), "{:?}", vertices[2]);
        assert_eq!(poly.validate(), Ok(()));
        // Off the plot entirely: rejected and unchanged.
        let mut far = before.clone();
        assert!(matches!(
            e.drag_by(&mut far, &NoMeasure, 1000., 0.),
            NudgeResult::Rejected(_)
        ));
        assert_eq!(far, before);
        // Two vertices are not a polygon.
        let two = shape(AnnotationKind::Polygon {
            vertices: vec![
                Anchor::Ratio { x: 0.1, y: 0.1 },
                Anchor::Ratio { x: 0.5, y: 0.1 },
            ],
            style: ShapeStyle::default(),
        });
        assert_eq!(
            two.validate(),
            Err("annotation polygon needs at least three vertices")
        );
    }
}
