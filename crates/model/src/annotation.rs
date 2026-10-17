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
//! exactly as bounded chart decorations do through `DecorationLayout`.
//!
//! Drags write anchors back through the same space, so the stored document
//! keeps data coordinates where the user placed them: a data-anchored item
//! follows zoom and range changes, a ratio-anchored item stays put.

use crate::color::Color;
use crate::decoration::DecorationId;
use crate::drag::{DragConfig, Draggable};
use crate::layer::AnnotationLayer;
use crate::layout::{NudgeReject, NudgeResult, RectF};
use crate::line::LineStylePreset;
use crate::resize::{HANDLE_SIZE, Resizable, ResizeHandle};
use crate::select::{SELECTION_COLOR, SELECTION_PADDING, SELECTION_STROKE_WIDTH, Selectable};
use crate::text::{MeasureText, RichText};

/// The most items one annotation layer accepts.
pub const MAX_ANNOTATIONS: usize = 1024;

/// A document axis. `X` is horizontal on screen and `Y` vertical in every
/// chart family: Cartesian charts map them to `bottom_x` / `left_y`, bounded
/// bar and box charts to the category index axis and the value axis
/// according to their direction, and radial charts have neither.
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

fn default_visible() -> bool {
    true
}

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

    /// Whether this item is drawn below the data (`Span`) or above every
    /// decoration (everything else).
    pub fn is_underlay(&self) -> bool {
        matches!(self.kind, AnnotationKind::Span { .. })
    }

    fn texts(&self) -> impl Iterator<Item = &RichText> {
        let (a, b) = match &self.kind {
            AnnotationKind::ReferenceLine { label, .. } => (label.as_ref(), None),
            AnnotationKind::Span { .. } => (None, None),
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

/// The rich text limits shared with bounded chart titles: font size 6..160,
/// at most 512 segments and eight lines, finite per-segment styles.
fn validate_rich_text(t: &RichText) -> Result<(), &'static str> {
    if !range(t.font_size, 6., 160.)
        || validate_color(t.color).is_err()
        || t.font.chars().count() > 160
        || t.font.chars().any(char::is_control)
        || t.segments.len() > 512
        || t.segments.iter().filter(|s| s.text == '\n').count() > 7
        || t.segments.iter().any(|s| {
            (s.text.is_control() && !matches!(s.text, '\n' | '\t'))
                || s.font_size.is_some_and(|v| !range(v, 6., 160.))
                || s.color.is_some_and(|v| validate_color(v).is_err())
                || s.field_em.is_some_and(|v| !range(v, 0., 32.))
                || s.rule_dash
                    .as_ref()
                    .is_some_and(|d| d.len() > 32 || d.iter().any(|v| !range(*v, 0.01, 32.)))
        })
    {
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
    /// No value axis (radial charts): only ratio anchors can be placed.
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
            AxisMap::Linear { min, max } => {
                let span = max - min;
                if !span.is_finite() || span == 0.0 {
                    return None;
                }
                (value - min) / span
            }
            AxisMap::Log { min, max } => {
                if value <= 0.0 || min <= 0.0 || max <= 0.0 {
                    return None;
                }
                let span = max.log10() - min.log10();
                if !span.is_finite() || span == 0.0 {
                    return None;
                }
                (value.log10() - min.log10()) / span
            }
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
/// chart-local for bounded charts and in the chart area's frame for
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
}

/// A registered annotation: the item index, the frame's anchor space and the
/// renderer-measured size of the item's text block (logical pixels, already
/// including box padding), so hit bounds use the same metrics as the raster
/// at every output scale.
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationElement {
    pub index: usize,
    pub space: PixelSpace,
    /// `[width, height]` of the text block, `None` without text.
    pub text_size: Option<[f32; 2]>,
    /// Set on the nudge-path copies a handle drag produces; `None` moves the
    /// whole item.
    pub handle: Option<AnnotationHandle>,
}

/// Half the thickness of the strip a reference line can be grabbed by.
const LINE_HIT_HALF_WIDTH: f32 = 3.0;
/// Gap between an arrow's shaft and its label.
pub const ARROW_LABEL_GAP: f32 = 6.0;
/// Inset of a reference-line label from the plot edge and the line.
pub const LINE_LABEL_INSET: f32 = 4.0;

/// Screen-space geometry of one item under a layer state, shared by the
/// hit bounds and the raster so they cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AnnotationGeometry {
    /// A reference line from `a` to `b` (axis-aligned, clipped to the plot).
    Line { a: [f32; 2], b: [f32; 2] },
    /// A span's filled rectangle, clipped to the plot.
    Span(RectF),
    /// A text block's rectangle (padded when boxed).
    Text(RectF),
    /// An arrow's endpoints.
    Arrow { from: [f32; 2], to: [f32; 2] },
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

    /// Where the item of `cfg` at this element's index is drawn, or `None`
    /// when it is hidden, absent, outside the plot or unplaceable.
    pub fn geometry(&self, cfg: &AnnotationLayer) -> Option<AnnotationGeometry> {
        let item = cfg.items.get(self.index)?;
        if !item.visible {
            return None;
        }
        geometry(&item.kind, &self.space, self.text_size)
    }

    /// The label rectangle of a reference line or an arrow, when it has one.
    pub fn label_rect(&self, cfg: &AnnotationLayer) -> Option<RectF> {
        let size = self.text_size?;
        match self.geometry(cfg)? {
            AnnotationGeometry::Line { a, b } => Some(line_label_rect(a, b, size)),
            AnnotationGeometry::Arrow { from, to } => Some(arrow_label_rect(from, to, size)),
            _ => None,
        }
    }

    /// The handles this item exposes, as compass names on its selection box
    /// and the part each one drags, with the square's centre.
    fn handles(&self, cfg: &AnnotationLayer) -> Vec<(ResizeHandle, AnnotationHandle, [f32; 2])> {
        let Some(geometry) = self.geometry(cfg) else {
            return Vec::new();
        };
        let Some(rect) = self.bounds(cfg, &NoMeasure) else {
            return Vec::new();
        };
        let rect = rect.expanded(SELECTION_PADDING);
        let (x0, y0) = (rect.x, rect.y);
        let (x1, y1) = (rect.x + rect.width, rect.y + rect.height);
        let (cx, cy) = (rect.x + rect.width * 0.5, rect.y + rect.height * 0.5);
        match geometry {
            AnnotationGeometry::Line { a, b } => {
                if a[0] == b[0] {
                    vec![
                        (ResizeHandle::W, AnnotationHandle::Value, [x0, cy]),
                        (ResizeHandle::E, AnnotationHandle::Value, [x1, cy]),
                    ]
                } else {
                    vec![
                        (ResizeHandle::N, AnnotationHandle::Value, [cx, y0]),
                        (ResizeHandle::S, AnnotationHandle::Value, [cx, y1]),
                    ]
                }
            }
            AnnotationGeometry::Span(_) => {
                let Some(Annotation {
                    kind: AnnotationKind::Span { axis, .. },
                    ..
                }) = cfg.items.get(self.index)
                else {
                    return Vec::new();
                };
                match axis {
                    AnnotationAxis::X => vec![
                        (ResizeHandle::W, AnnotationHandle::Start, [x0, cy]),
                        (ResizeHandle::E, AnnotationHandle::End, [x1, cy]),
                    ],
                    AnnotationAxis::Y => vec![
                        (ResizeHandle::N, AnnotationHandle::Start, [cx, y0]),
                        (ResizeHandle::S, AnnotationHandle::End, [cx, y1]),
                    ],
                }
            }
            AnnotationGeometry::Text(_) => Vec::new(),
            AnnotationGeometry::Arrow { from, to } => {
                let (from_name, to_name) = arrow_handle_names(from, to);
                vec![
                    (from_name, AnnotationHandle::From, from),
                    (to_name, AnnotationHandle::To, to),
                ]
            }
        }
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

/// The label of a reference line: inset from the line's top (vertical) or
/// left (horizontal) end, on the line's right or upper side.
pub fn line_label_rect(a: [f32; 2], b: [f32; 2], size: [f32; 2]) -> RectF {
    if a[0] == b[0] {
        RectF {
            x: a[0] + LINE_LABEL_INSET,
            y: a[1].min(b[1]) + LINE_LABEL_INSET,
            width: size[0],
            height: size[1],
        }
    } else {
        RectF {
            x: a[0].min(b[0]) + LINE_LABEL_INSET,
            y: a[1] - LINE_LABEL_INSET - size[1],
            width: size[0],
            height: size[1],
        }
    }
}

/// The label of an arrow: centred on the shaft's midpoint, pushed off the
/// shaft towards the top of the screen by [`ARROW_LABEL_GAP`].
pub fn arrow_label_rect(from: [f32; 2], to: [f32; 2], size: [f32; 2]) -> RectF {
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
    let reach = ARROW_LABEL_GAP + size[0].abs() * 0.5 * n[0].abs() + size[1] * 0.5 * n[1].abs();
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
    }
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

/// Hit bounds for a geometry: a thin strip for lines, the rectangle for
/// spans and text, the endpoint bounding box plus the head for arrows. The
/// label of a line or arrow is not part of the item's bounds.
pub fn geometry_bounds(geometry: &AnnotationGeometry, head_half_width: f32) -> RectF {
    match *geometry {
        AnnotationGeometry::Line { a, b } => RectF {
            x: a[0].min(b[0]) - LINE_HIT_HALF_WIDTH,
            y: a[1].min(b[1]) - LINE_HIT_HALF_WIDTH,
            width: (a[0] - b[0]).abs() + LINE_HIT_HALF_WIDTH * 2.0,
            height: (a[1] - b[1]).abs() + LINE_HIT_HALF_WIDTH * 2.0,
        },
        AnnotationGeometry::Span(rect) | AnnotationGeometry::Text(rect) => rect,
        AnnotationGeometry::Arrow { from, to } => {
            let pad = head_half_width.max(LINE_HIT_HALF_WIDTH);
            RectF {
                x: from[0].min(to[0]) - pad,
                y: from[1].min(to[1]) - pad,
                width: (from[0] - to[0]).abs() + pad * 2.0,
                height: (from[1] - to[1]).abs() + pad * 2.0,
            }
        }
    }
}

fn head_half_width(kind: &AnnotationKind) -> f32 {
    match kind {
        AnnotationKind::Arrow { style, .. } => style.head_width.max(style.line.width) * 0.5,
        _ => 0.0,
    }
}

/// Annotation bounds never depend on glyph metrics at hit time: the renderer
/// measured the text when it registered the element.
struct NoMeasure;
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
    fn bounds(&self, cfg: &AnnotationLayer, _: &dyn MeasureText) -> Option<RectF> {
        let item = cfg.items.get(self.index)?;
        let geometry = self.geometry(cfg)?;
        Some(geometry_bounds(&geometry, head_half_width(&item.kind)))
    }

    /// The common blue box, carrying only this item's own handles: a span's
    /// two edges, a reference line's two value handles, an arrow's two
    /// endpoints, none for a text box.
    fn selection_box(
        &self,
        cfg: &AnnotationLayer,
        measure: &dyn MeasureText,
    ) -> Option<crate::select::SelectionBox> {
        let rect = self.bounds(cfg, measure)?.expanded(SELECTION_PADDING);
        Some(crate::select::SelectionBox {
            rect,
            color: SELECTION_COLOR,
            stroke_width: SELECTION_STROKE_WIDTH,
            handles: self
                .handles(cfg)
                .into_iter()
                .map(|(_, _, center)| handle_square(center))
                .collect(),
        })
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
    /// handles [`Selectable::selection_box`] shows are live; `resize_by`
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

    fn hit_resize_handle(
        &self,
        cfg: &AnnotationLayer,
        _: &dyn MeasureText,
        x: f32,
        y: f32,
    ) -> Option<ResizeHandle> {
        self.handles(cfg)
            .into_iter()
            .find(|(_, _, center)| handle_square(*center).contains(x, y))
            .map(|(name, _, _)| name)
    }

    /// Arrow endpoints take both motion components through one nudge, so a
    /// diagonal handle drag is a single accept-or-reject decision.
    fn resize_by(
        &self,
        cfg: &mut AnnotationLayer,
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
                cfg.nudge(self.with_handle(part), dx, dy)
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
                cfg.nudge(self.with_handle(part), dx, dy)
            }
            AnnotationKind::Text { .. } => reject,
        }
    }
}

impl DragConfig for AnnotationLayer {
    type Element = AnnotationElement;

    /// Move an item or one of its handles by a screen delta. Values and
    /// anchors are converted back through the element's space, keeping
    /// their anchor kind. A move is rejected atomically when the result
    /// would be unplaceable: a line or span edge outside the axis range, a
    /// span entirely outside the plot, a text box or arrow endpoint outside
    /// the panel.
    fn nudge(&mut self, element: AnnotationElement, dx: f32, dy: f32) -> NudgeResult {
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
            let next = [p[0] + dx, p[1] + dy];
            if !space.panel.contains(next[0], next[1]) {
                return None;
            }
            space.px_to_anchor(next, anchor.kind())
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
                let Some(v) = moved(*value, *axis).filter(|v| in_range(*v, *axis)) else {
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
                        let (Some(ta), Some(tb)) =
                            (space.fraction(*axis, na), space.fraction(*axis, nb))
                        else {
                            return reject;
                        };
                        if ta.max(tb) < 0.0 || ta.min(tb) > 1.0 {
                            return reject;
                        }
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
                let (Some(p), Some(size)) = (space.anchor_to_px(anchor), element.text_size) else {
                    return reject;
                };
                let rect = RectF {
                    x: p[0] + dx,
                    y: p[1] + dy,
                    width: size[0],
                    height: size[1],
                };
                let Some(next) = inside_panel(&space, &rect)
                    .then(|| space.px_to_anchor([rect.x, rect.y], anchor.kind()))
                    .flatten()
                else {
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
            _ => return reject,
        };
        self.items[element.index].kind = next;
        NudgeResult::Moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn element(index: usize, text_size: Option<[f32; 2]>) -> AnnotationElement {
        AnnotationElement {
            index,
            space: space(),
            text_size,
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
        assert_eq!(line.drag_by(&mut layer, 30., 99.), NudgeResult::Moved);
        match layer.items[0].kind {
            AnnotationKind::ReferenceLine { value, .. } => assert!((value - 3.).abs() < 1e-5),
            _ => unreachable!(),
        }
        assert!(matches!(
            line.drag_by(&mut layer, 1000., 0.),
            NudgeResult::Rejected(_)
        ));
        assert!(matches!(
            line.drag_by(&mut layer, f32::NAN, 0.),
            NudgeResult::Rejected(_)
        ));
        // A vertical line's live handles are W and E; N drives nothing.
        assert_eq!(
            line.resize_by(&mut layer, ResizeHandle::E, 30., 0.),
            NudgeResult::Moved
        );
        assert!(matches!(
            line.resize_by(&mut layer, ResizeHandle::N, 0., 30.),
            NudgeResult::Rejected(_)
        ));

        let span = element(1, None);
        assert_eq!(span.drag_by(&mut layer, 30., 0.), NudgeResult::Moved);
        match layer.items[1].kind {
            AnnotationKind::Span { from, to, .. } => {
                assert!((from - 7.).abs() < 1e-5 && (to - 5.).abs() < 1e-5)
            }
            _ => unreachable!(),
        }
        // `Start` is the left edge in screen order: here `to`, the smaller value.
        assert_eq!(
            span.resize_by(&mut layer, ResizeHandle::W, -30., 0.),
            NudgeResult::Moved
        );
        match layer.items[1].kind {
            AnnotationKind::Span { from, to, .. } => {
                assert!((from - 7.).abs() < 1e-5 && (to - 4.).abs() < 1e-5)
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            span.drag_by(&mut layer, 400., 0.),
            NudgeResult::Rejected(_)
        ));

        let text_box = element(2, Some([40., 14.]));
        let p0 = space()
            .anchor_to_px(&Anchor::Data { x: 5., y: 10. })
            .unwrap();
        assert_eq!(text_box.drag_by(&mut layer, 10., -20.), NudgeResult::Moved);
        match layer.items[2].kind {
            AnnotationKind::Text { anchor, .. } => {
                let p = space().anchor_to_px(&anchor).unwrap();
                assert!((p[0] - p0[0] - 10.).abs() < 1e-3 && (p[1] - p0[1] + 20.).abs() < 1e-3);
                assert_eq!(anchor.kind(), AnchorKind::Data);
            }
            _ => unreachable!(),
        }
        // Anywhere in the panel, but not beyond it.
        assert_eq!(text_box.drag_by(&mut layer, 150., 0.), NudgeResult::Moved);
        assert!(matches!(
            text_box.drag_by(&mut layer, 100., 0.),
            NudgeResult::Rejected(_)
        ));

        let arrow = element(3, None);
        assert_eq!(arrow.drag_by(&mut layer, 10., 10.), NudgeResult::Moved);
        assert_eq!(
            arrow.resize_by(&mut layer, ResizeHandle::E, 5., -5.),
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
            arrow.resize_by(&mut layer, ResizeHandle::W, -1000., 0.),
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
        assert_eq!(sb.handles.len(), 2);
        let top = sb.handles[0];
        assert_eq!(
            span.hit_resize_handle(&layer, &NoMeasure, top.x + 4., top.y + 4.),
            Some(ResizeHandle::N)
        );
        assert_eq!(
            span.hit_resize_handle(&layer, &NoMeasure, sb.rect.x, sb.rect.y),
            None
        );
        let text_box = element(1, Some([10., 10.]));
        assert!(
            text_box
                .selection_box(&layer, &NoMeasure)
                .unwrap()
                .handles
                .is_empty()
        );
        let arrow = element(2, None);
        let sb = arrow.selection_box(&layer, &NoMeasure).unwrap();
        assert_eq!(sb.handles.len(), 2);
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
    }
}
