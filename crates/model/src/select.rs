//! Selection policy — `Selectable` and the chart elements that implement it.
//!
//! Selectable objects expose their interaction-policy pixel bounds (`bounds`); what
//! selection *does* is fixed by the trait's default methods so it applies
//! uniformly to every element: a blue highlight box around the bounds
//! (`selection_box`) and point hit-testing (`contains`). The renderer draws
//! the box with skia (`axis_render::draw_selection_boxes`); the model only
//! states the policy.
//!
//! Placement formulas shared with the renderer consume glyph extents through
//! the [`MeasureText`] contract. Tick labels and axis strokes come from the
//! renderer's own layout ([`MeasureText::drawn_parts`]): tick generation,
//! formatting and pruning belong to the renderer, and the model does not
//! estimate them.

use crate::color::Color;
use crate::config::{AxisOptions, Config};
use crate::decoration::DecorationId;
use crate::drag::{DragConfig, Draggable};
use crate::layer::LayerRef;
use crate::layout::{
    DrawnPart, Element, NudgeReject, NudgeResult, RectF, Side, TitleBand, axis_title_placement,
    chart_title_placement, colorbar_rect, colorbar_title_placement, legend_rect,
};
use crate::resize::Resizable;
use crate::text::MeasureText;

/// Selection highlight color (blue).
pub const SELECTION_COLOR: Color = Color {
    r: 0.13,
    g: 0.47,
    b: 0.95,
    a: 1.0,
};
/// Stroke width of the highlight box, px.
pub const SELECTION_STROKE_WIDTH: f32 = 1.5;
/// Gap between the element bounds and the highlight box, px.
pub const SELECTION_PADDING: f32 = 2.0;

/// What a selection highlight outlines.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SelectionOutline {
    /// The box [`SelectionBox::rect`].
    #[default]
    Rect,
    /// Each tick label of the axis on this side, boxed where the renderer
    /// draws it, in place of `rect`, which still bounds them. Polar tick
    /// labels ring the plot or line the ray, so one box around them would
    /// cover the chart. The model does not lay out tick labels; a consumer
    /// without that layout draws `rect`.
    TickLabels(Side),
}

/// The blue box drawn around a selected element. Produced by
/// [`Selectable::selection_box`]; consumed by the renderer's skia pass.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionBox {
    /// Box rect in chart-surface pixels (already padding-expanded).
    pub rect: RectF,
    pub color: Color,
    pub stroke_width: f32,
    /// Resize handle squares (empty for non-resizable elements). Drawn as
    /// filled squares on top of the box outline.
    pub handles: Vec<RectF>,
    /// What the highlight outlines: `rect`, or each tick label of an axis.
    pub outline: SelectionOutline,
}

/// An object the user can select on the chart.
///
/// Implementations provide [`Self::bounds`] only. Selection behavior lives in
/// the default methods so it is identical across elements; override them only
/// to change the policy itself.
pub trait Selectable<C: DragConfig = Config> {
    /// Interaction bounds of this element under `cfg`, or `None` when the
    /// element is hidden, empty, or the layout is infeasible. Text-bearing
    /// elements use `measure`; tick labels and axes are the parts `measure`
    /// reports as drawn, and have no bounds under a measure that does not
    /// lay them out.
    fn bounds(&self, cfg: &C, measure: &dyn MeasureText) -> Option<RectF>;

    /// Default selection policy: a blue box [`SELECTION_PADDING`] px outside
    /// the bounds, stroked [`SELECTION_STROKE_WIDTH`] px in
    /// [`SELECTION_COLOR`]. Resizable elements additionally carry the eight
    /// resize handle squares.
    fn selection_box(&self, cfg: &C, measure: &dyn MeasureText) -> Option<SelectionBox> {
        let rect = self.bounds(cfg, measure)?.expanded(SELECTION_PADDING);
        let handles = match self.as_resizable() {
            Some(_) => crate::resize::handle_rects(&rect)
                .iter()
                .map(|(_, r)| *r)
                .collect(),
            None => Vec::new(),
        };
        Some(SelectionBox {
            rect,
            color: SELECTION_COLOR,
            stroke_width: SELECTION_STROKE_WIDTH,
            handles,
            outline: self.selection_outline(cfg),
        })
    }

    /// What the highlight outlines; every element but polar tick labels
    /// outlines its bounds.
    fn selection_outline(&self, _cfg: &C) -> SelectionOutline {
        SelectionOutline::Rect
    }

    /// Default hit test: point-in-bounds.
    fn contains(&self, cfg: &C, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        self.bounds(cfg, measure).is_some_and(|b| b.contains(x, y))
    }

    /// Default registration: every `Selectable` can enter a [`HitMap`] the
    /// same way. Returns the id the map will report from `hit_test`.
    /// (`Send + Sync` because hosts store hit maps in cross-thread render
    /// state, e.g. egui's `CallbackResources`.)
    fn register_into(self, map: &mut HitMap<C>) -> HitId
    where
        Self: Sized + Send + Sync + 'static,
    {
        map.register(self)
    }

    /// This element's drag capability, if any. Elements that also implement
    /// [`Draggable`] override this with `Some(self)`. Lets hosts go from a
    /// [`HitMap`] hit straight to dragging without knowing concrete types.
    fn as_draggable(&self) -> Option<&dyn Draggable<C>> {
        None
    }

    /// This element's resize capability, if any. Elements that also implement
    /// [`Resizable`] override this with `Some(self)` — their selection box
    /// then grows the eight resize handles automatically.
    fn as_resizable(&self) -> Option<&dyn Resizable<C>> {
        None
    }

    /// Stable element name for hosts that key on identity rather than
    /// [`HitId`] (e.g. a wasm host exposing hit-testing as strings):
    /// `"data_area"`, `"axis_bottom"`, `"tick_labels_left"`,
    /// `"axis_title_left"`, `"legend"`, `"chart_title"`,
    /// `"colorbar_axis"`, `"colorbar_tick_labels"`, `"colorbar_title"`.
    /// Custom host elements keep the default.
    fn element_id(&self) -> String {
        "custom".to_string()
    }
}

fn side_str(side: &Side) -> &'static str {
    match side {
        Side::Top => "top",
        Side::Bottom => "bottom",
        Side::Left => "left",
        Side::Right => "right",
    }
}

/// Id of an entry registered in a [`HitMap`]. Stable for the map's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitId(usize);

/// Registry of selectable elements for one chart panel + point hit-testing.
///
/// Hosts feed pointer events here: `hit_test` returns the topmost registered
/// element under the point (later registrations win — register background
/// elements like the data area first, small foreground elements like titles
/// last). The model owns this because hit-testing is pure bounds geometry;
/// the renderer only supplies the [`MeasureText`] implementation.
pub struct HitMap<C: DragConfig = Config> {
    entries: Vec<Box<dyn Selectable<C> + Send + Sync>>,
}

impl<C: DragConfig> HitMap<C> {
    /// Create a map for an explicit state type; Cartesian callers keep `new()`.
    pub fn for_state() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn register(&mut self, el: impl Selectable<C> + Send + Sync + 'static) -> HitId {
        self.entries.push(Box::new(el));
        HitId(self.entries.len() - 1)
    }

    /// Topmost element containing `(x, y)` — entries are tested in reverse
    /// registration order.
    pub fn hit_test(&self, cfg: &C, measure: &dyn MeasureText, x: f32, y: f32) -> Option<HitId> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .find(|(_, el)| el.contains(cfg, measure, x, y))
            .map(|(i, _)| HitId(i))
    }

    pub fn get(&self, id: HitId) -> Option<&(dyn Selectable<C> + Send + Sync)> {
        self.entries.get(id.0).map(|b| b.as_ref())
    }

    /// Resolve a stable element name without reconstructing renderer geometry.
    pub fn find(&self, name: &str) -> Option<HitId> {
        self.entries
            .iter()
            .position(|e| e.element_id() == name)
            .map(HitId)
    }

    /// Selection highlight for a registered element — the registered
    /// element's [`Selectable::selection_box`] policy.
    pub fn selection_box(
        &self,
        id: HitId,
        cfg: &C,
        measure: &dyn MeasureText,
    ) -> Option<SelectionBox> {
        self.get(id)?.selection_box(cfg, measure)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for HitMap<Config> {
    fn default() -> Self {
        Self::new()
    }
}

impl HitMap<Config> {
    pub fn new() -> Self {
        Self::for_state()
    }
    /// The standard registration set for a single chart panel, back-to-front:
    /// data area → axes → tick-label bands → axis titles → legend → chart
    /// title → colourbar strip → colourbar axis → colourbar labels →
    /// colourbar title.
    ///
    /// Colourbar chrome is last, so it wins over anything it overlaps. Its
    /// smaller parts follow the strip, allowing the ticks, labels, and title to
    /// be selected independently while the strip remains the resize target.
    pub fn standard_chart() -> Self {
        let mut map = Self::new();
        map.register(DataAreaElement);
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisElement { side });
        }
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisLabelElement { side });
        }
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            map.register(AxisTitleElement { side });
        }
        map.register(LegendElement);
        map.register(ChartTitleElement);
        map.register(ColorBarElement);
        map.register(ColorBarAxisElement);
        map.register(ColorBarLabelElement);
        map.register(ColorBarTitleElement);
        map
    }

    /// The registration set for the layer `cfg.active_layer()` addresses.
    ///
    /// `Base` is exactly [`Self::standard_chart`]. For any other layer the
    /// shared elements come first — data area, the two X axes with their
    /// label bands and titles, the merged legend, the chart title and the
    /// colourbar chrome — and the active layer's own elements last, so the
    /// reverse-order hit test puts them on top: for `Overlay(i)` its right
    /// Y axis, that axis' label band and title, and its per-layer legend
    /// (`DecorationId::Overlay { layer: i, .. }` ids, `overlay_<i>:…`).
    /// The inactive layers' elements are not registered at all, so a click
    /// on them passes through to the active layer; the base panel's Y axes
    /// are therefore absent while an overlay or the annotation layer is
    /// active. For `Annotations` the map holds the shared elements only:
    /// the annotation items live in their own renderer-measured
    /// `HitMap<AnnotationLayer>`, and the pointer route (the renderer's
    /// `ChartView::hit_test_active_layer`) tests them on top of this map
    /// while that layer is active and skips them on every other layer.
    ///
    /// `Subplot(i)` and `SubplotOverlay(i, j)` register exactly the set that
    /// subplot's own chart registers for `Base` or `Overlay(j)`, wrapped as
    /// [`SubplotElement`]s (`subplot_<i>:…` ids); the parent's elements are
    /// not registered then, since a subplot shares none of them.
    ///
    /// The set depends only on the active layer, the overlay count and the
    /// active subplot's overlay count, so a host rebuilds the map when any
    /// of them changes; `HitId`s from a previous map are not valid in the new
    /// one.
    pub fn for_active_layer(cfg: &Config) -> Self {
        let layer = cfg.active_layer();
        if layer == LayerRef::Base {
            return Self::standard_chart();
        }
        if let LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) = layer {
            return Self::for_subplot(cfg, index, layer);
        }
        let mut map = Self::new();
        map.register(DataAreaElement);
        for side in [Side::Top, Side::Bottom] {
            map.register(AxisElement { side });
        }
        for side in [Side::Top, Side::Bottom] {
            map.register(AxisLabelElement { side });
        }
        for side in [Side::Top, Side::Bottom] {
            map.register(AxisTitleElement { side });
        }
        map.register(LegendElement);
        map.register(ChartTitleElement);
        map.register(ColorBarElement);
        map.register(ColorBarAxisElement);
        map.register(ColorBarLabelElement);
        map.register(ColorBarTitleElement);
        if let LayerRef::Overlay(index) = layer {
            map.register(OverlayAxisElement { layer: index });
            map.register(OverlayAxisLabelElement { layer: index });
            map.register(OverlayAxisTitleElement { layer: index });
            map.register(OverlayLegendElement { layer: index });
        }
        map
    }
}

impl HitMap<Config> {
    /// The registration set of an active subplot: exactly the set its own
    /// chart registers for its own active layer (`Base`, or `Overlay(j)` for
    /// `SubplotOverlay(index, j)`), each element wrapped in a
    /// [`SubplotElement`]. A subplot shares no element with the parent, so
    /// nothing else is registered.
    fn for_subplot(cfg: &Config, index: usize, layer: LayerRef) -> Self {
        let Some(mut subplot) = cfg.subplot_config(index) else {
            return Self::standard_chart();
        };
        subplot.active_layer = match layer {
            LayerRef::SubplotOverlay(_, overlay) => LayerRef::Overlay(overlay),
            _ => LayerRef::Base,
        };
        let inner = Self::for_active_layer(&subplot);
        let mut map = Self::new();
        for element in inner.entries {
            map.register(SubplotElement {
                subplot: index,
                inner: element,
            });
        }
        map
    }
}

/// An element of subplot `subplot`'s own chart, registered while that
/// subplot (or one of its overlays) is the active layer.
///
/// It measures against the subplot's chart ([`Config::subplot_config`]) and
/// applies a drag or resize to that chart through the element's own policy,
/// then stores the result in `subplots[subplot].config`, so every rule of the
/// single chart (data-area margins, axis offsets, overlay axes, legends)
/// holds unchanged inside a subplot. The id is the inner id prefixed with
/// `subplot_<n>:`.
pub struct SubplotElement {
    pub subplot: usize,
    inner: Box<dyn Selectable + Send + Sync>,
}

impl std::fmt::Debug for SubplotElement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubplotElement")
            .field("subplot", &self.subplot)
            .field("inner", &self.inner.element_id())
            .finish()
    }
}

impl SubplotElement {
    /// Run `edit` on the subplot's chart and store what it moved. The linked
    /// axis ranges stay their targets' and the subplot's own active layer
    /// and annotation layer are not part of the edit.
    fn edit(&self, cfg: &mut Config, edit: impl FnOnce(&mut Config) -> NudgeResult) -> NudgeResult {
        let Some(mut chart) = cfg.subplot_config(self.subplot) else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let result = edit(&mut chart);
        if result == NudgeResult::Moved {
            let stored = &mut cfg.subplots[self.subplot].config;
            chart.active_layer = stored.active_layer;
            chart.annotations = std::mem::take(&mut stored.annotations);
            *stored = chart;
        }
        result
    }
}

impl Selectable for SubplotElement {
    fn element_id(&self) -> String {
        format!("subplot_{}:{}", self.subplot, self.inner.element_id())
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        self.inner
            .bounds(&cfg.subplot_config(self.subplot)?, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        self.inner.as_draggable().map(|_| self as &dyn Draggable)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        self.inner.as_resizable().map(|_| self as &dyn Resizable)
    }
}

impl Draggable for SubplotElement {
    fn nudge_element(&self) -> Element {
        self.inner
            .as_draggable()
            .map_or(Element::DataArea, |drag| drag.nudge_element())
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| drag.drag_by(chart, dx, dy))
    }

    fn drag_from(&self, cfg: &mut Config, at: [f32; 2], dx: f32, dy: f32) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        // A subplot's chart keeps document coordinates, so the pointer
        // passes through unchanged.
        self.edit(cfg, |chart| drag.drag_from(chart, at, dx, dy))
    }
}

impl Resizable for SubplotElement {
    fn resize_targets(
        &self,
        handle: crate::resize::ResizeHandle,
    ) -> (Option<Element>, Option<Element>) {
        self.inner
            .as_resizable()
            .map_or((None, None), |resize| resize.resize_targets(handle))
    }

    fn resize_by(
        &self,
        cfg: &mut Config,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(resize) = self.inner.as_resizable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| resize.resize_by(chart, handle, dx, dy))
    }

    fn resize_handles(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
    ) -> Option<[(crate::resize::ResizeHandle, RectF); 8]> {
        self.inner
            .as_resizable()?
            .resize_handles(&cfg.subplot_config(self.subplot)?, measure)
    }
}

// Selectable chart elements.

/// One of the four axes — the axis line plus its tick band.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisElement {
    pub side: Side,
}

/// One axis title (rotated 90° on Left/Right sides).
#[derive(Debug, Clone, PartialEq)]
pub struct AxisTitleElement {
    pub side: Side,
}

/// One axis's tick-value labels — the band the numeric labels live in,
/// between the tick ends and the outer chart edge.
#[derive(Debug, Clone, PartialEq)]
pub struct AxisLabelElement {
    pub side: Side,
}

/// The chart title in the top band.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartTitleElement;

/// The data plotting area (everything inside the margins).
#[derive(Debug, Clone, PartialEq)]
pub struct DataAreaElement;

/// The legend box in one corner of the data area.
#[derive(Debug, Clone, PartialEq)]
pub struct LegendElement;

/// The right Y axis of overlay layer `layer` — line plus tick band.
///
/// Overlay elements measure against the overlay's internal panel
/// (`Config::overlay_panel`), whose data area is the group data area, and
/// write a drag back into `overlays[layer].y_axis` only, so the parent's own
/// right axis never moves on their behalf.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisElement {
    pub layer: usize,
}

/// The tick-label band of overlay layer `layer`'s Y axis.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisLabelElement {
    pub layer: usize,
}

/// The title of overlay layer `layer`'s Y axis.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisTitleElement {
    pub layer: usize,
}

/// The per-layer legend of overlay layer `layer` (`OverlayLayer::legend`);
/// no bounds while the overlay has none.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLegendElement {
    pub layer: usize,
}

/// The colourbar strip.
///
/// Carries no side of its own: the bar's `side` lives in `Config.colorbar`, and
/// a copy here would be a mirror that can disagree with what is drawn. That is
/// also why the resize handles map to screen-side edges
/// ([`Element::ColorBarEdge`]) and let
/// nudge resolve which dimension each one drives.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorBarElement;

/// The colourbar axis line plus its major/minor tick band.
///
/// Like [`ColorBarElement`], this carries no mirrored `Side`; orientation is
/// always read from `Config.colorbar` at the point of use.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorBarAxisElement;

/// The colourbar's tick-value label band.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorBarLabelElement;

/// The colourbar title, rotated on vertical bars.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorBarTitleElement;

fn axis_of<'a>(cfg: &'a Config, side: &Side) -> &'a AxisOptions {
    match side {
        Side::Top => &cfg.top_x,
        Side::Bottom => &cfg.bottom_x,
        Side::Left => &cfg.left_y,
        Side::Right => &cfg.right_y,
    }
}

/// The config the renderer draws the base layer of a layered document with
/// (the expanded base panel, or the document without its layers when it does
/// not expand at this size, as the renderer's panel expansion does), or `None`
/// for a single-layer document, which is drawn as it is.
fn drawn_base_panel(cfg: &Config) -> Option<Config> {
    if cfg.overlays.is_empty() && cfg.subplots.is_empty() {
        return None;
    }
    match cfg.expand() {
        Ok(expansion) => expansion
            .panel(LayerRef::Base)
            .map(|panel| panel.config.clone()),
        Err(_) => {
            let mut panel = cfg.clone();
            panel.overlays.clear();
            panel.subplots.clear();
            panel.active_layer = LayerRef::Base;
            Some(panel)
        }
    }
}

/// The extent of a drawn part: a label's glyph block, or a segment's stroke
/// with at least `min_half` either side.
fn part_extent(part: &DrawnPart, min_half: f32) -> RectF {
    match *part {
        DrawnPart::Text(r) => r,
        DrawnPart::Segment { a, b, width } => {
            let half = (width * 0.5).max(min_half);
            RectF {
                x: a[0].min(b[0]) - half,
                y: a[1].min(b[1]) - half,
                width: (a[0] - b[0]).abs() + half * 2.0,
                height: (a[1] - b[1]).abs() + half * 2.0,
            }
        }
    }
}

/// Whether `(x, y)` is on a drawn part: inside a label's block grown by
/// [`SELECTION_PADDING`], as its outline is, or within half a segment's
/// stroke and at least `min_half` of it.
fn part_contains(part: &DrawnPart, min_half: f32, x: f32, y: f32) -> bool {
    match *part {
        DrawnPart::Text(r) => r.expanded(SELECTION_PADDING).contains(x, y),
        DrawnPart::Segment { a, b, width } => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let len2 = d[0] * d[0] + d[1] * d[1];
            let t = if len2 > 0.0 {
                (((x - a[0]) * d[0] + (y - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (x - a[0] - t * d[0]).hypot(y - a[1] - t * d[1]) <= (width * 0.5).max(min_half)
        }
    }
}

/// The union of the parts of `element` the renderer draws under `cfg`,
/// laid out through `measure` ([`MeasureText::drawn_parts`]): `Some(None)`
/// when it draws none, `None` when `measure` cannot lay them out.
fn drawn_union(
    cfg: &Config,
    measure: &dyn MeasureText,
    element: &Element,
    min_half: f32,
) -> Option<Option<RectF>> {
    let mut union: Option<RectF> = None;
    let laid_out = measure.drawn_parts(cfg, element, &mut |part| {
        let r = part_extent(&part, min_half);
        union = Some(match union {
            None => r,
            Some(u) => {
                let (x0, y0) = (u.x.min(r.x), u.y.min(r.y));
                let x1 = (u.x + u.width).max(r.x + r.width);
                let y1 = (u.y + u.height).max(r.y + r.height);
                RectF {
                    x: x0,
                    y: y0,
                    width: x1 - x0,
                    height: y1 - y0,
                }
            }
        });
    });
    laid_out.then_some(union)
}

/// Whether `(x, y)` falls on one of the drawn parts of `element`
/// ([`part_contains`]), or `None` when `measure` cannot lay them out.
fn drawn_part_hit(
    cfg: &Config,
    measure: &dyn MeasureText,
    element: &Element,
    min_half: f32,
    x: f32,
    y: f32,
) -> Option<bool> {
    let mut hit = false;
    let laid_out = measure.drawn_parts(cfg, element, &mut |part| {
        hit |= part_contains(&part, min_half, x, y);
    });
    laid_out.then_some(hit)
}

/// Whether `(x, y)` falls inside the selection box around the drawn parts
/// of `element`, or `None` when `measure` cannot lay them out. The hit
/// region is the box the selection draws, so a click inside the box always
/// selects the element.
fn drawn_box_hit(
    cfg: &Config,
    measure: &dyn MeasureText,
    element: &Element,
    x: f32,
    y: f32,
) -> Option<bool> {
    let union = drawn_union(cfg, measure, element, 0.0)?;
    Some(union.is_some_and(|r| r.expanded(SELECTION_PADDING).contains(x, y)))
}

fn visible_colorbar_rect(cfg: &Config) -> Option<(&crate::config::ColorBarOptions, RectF)> {
    let bar = cfg.colorbar.as_ref()?;
    if !bar.visible {
        return None;
    }
    let da = cfg.data_area().ok()?;
    let rect = colorbar_rect(&cfg.chart_area, &da, cfg.chart_title.top_margin, bar);
    (rect.width > 0.0 && rect.height > 0.0).then_some((bar, rect))
}

/// Half the width of the hit strip along a polar ray, in logical pixels.
const POLAR_RAY_HIT_HALF_WIDTH: f32 = 4.0;

impl Selectable for AxisElement {
    fn element_id(&self) -> String {
        format!("axis_{}", side_str(&self.side))
    }

    /// The line and ticks the renderer draws ([`MeasureText::drawn_parts`]):
    /// a polar chart's angle axis is the outer ring, which is the data
    /// boundary and not selectable, and its radius ray is grabbed at least
    /// [`POLAR_RAY_HIT_HALF_WIDTH`] either side. `None` when nothing is
    /// drawn or `measure` does not lay the axis out.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        if cfg.polar.is_some() && self.side != Side::Left {
            return None;
        }
        let min_half = if cfg.polar.is_some() {
            POLAR_RAY_HIT_HALF_WIDTH
        } else {
            0.0
        };
        drawn_union(cfg, measure, &Element::Axis(self.side.clone()), min_half).flatten()
    }

    /// The polar ray hits on each drawn stroke, a Cartesian axis inside the
    /// selection box around its drawn line and ticks.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        let element = Element::Axis(self.side.clone());
        if cfg.polar.is_some() {
            self.side == Side::Left
                && drawn_part_hit(cfg, measure, &element, POLAR_RAY_HIT_HALF_WIDTH, x, y)
                    .unwrap_or(false)
        } else {
            drawn_box_hit(cfg, measure, &element, x, y).unwrap_or(false)
        }
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisLabelElement {
    fn element_id(&self) -> String {
        format!("tick_labels_{}", side_str(&self.side))
    }

    /// The union of the labels the renderer draws
    /// ([`MeasureText::drawn_parts`]); `None` when it draws none or
    /// `measure` does not lay tick labels out.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        drawn_union(cfg, measure, &Element::AxisLabel(self.side.clone()), 0.0).flatten()
    }

    /// Polar tick labels ring the plot (angle) or line the ray (radius):
    /// outline each label rather than the box around all of them.
    fn selection_outline(&self, cfg: &Config) -> SelectionOutline {
        match self.side {
            Side::Bottom | Side::Left if cfg.polar.is_some() => {
                SelectionOutline::TickLabels(self.side.clone())
            }
            _ => SelectionOutline::Rect,
        }
    }

    /// Polar labels hit on each drawn label, where each is outlined;
    /// Cartesian labels inside the one box around them.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        let labels = Element::AxisLabel(self.side.clone());
        let drawn = if cfg.polar.is_some() {
            drawn_part_hit(cfg, measure, &labels, 0.0, x, y)
        } else {
            drawn_box_hit(cfg, measure, &labels, x, y)
        };
        drawn.unwrap_or(false)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisTitleElement {
    fn element_id(&self) -> String {
        format!("axis_title_{}", side_str(&self.side))
    }

    /// Placed on the config the base layer is drawn with: in a layered
    /// document that is the expanded base panel, whose margins follow the
    /// widest axis band of the group.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        // A polar chart draws no top or right axis, so their titles have no
        // hit region either.
        if cfg.polar.is_some() && matches!(self.side, Side::Top | Side::Right) {
            return None;
        }
        let panel = drawn_base_panel(cfg);
        let cfg = panel.as_ref().unwrap_or(cfg);
        let axis = axis_of(cfg, &self.side);
        let to = &axis.title_option;
        if !to.visible || to.text.segments.is_empty() {
            return None;
        }
        let da = cfg.data_area().ok()?;
        let m = measure.measure_rich(&to.text);
        Some(
            axis_title_placement(
                self.side.clone(),
                &cfg.chart_area,
                &da,
                TitleBand {
                    out_margin: axis.out_margin,
                    chart_title_margin: cfg.chart_title.top_margin,
                    edge_inset: cfg.colorbar_band(&self.side),
                },
                (to.offset_x, to.offset_y),
                m,
            )
            .rect(m),
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ChartTitleElement {
    fn element_id(&self) -> String {
        "chart_title".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let ct = &cfg.chart_title;
        if !ct.visible || ct.text.segments.is_empty() {
            return None;
        }
        let m = measure.measure_rich(&ct.text);
        Some(
            chart_title_placement(
                &cfg.chart_area,
                ct.top_margin,
                (ct.offset_x, ct.offset_y),
                m,
            )
            .rect(m),
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for DataAreaElement {
    fn element_id(&self) -> String {
        "data_area".to_string()
    }

    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        cfg.data_area().ok().map(|da| RectF::from_rect(&da.0))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }
}

impl Selectable for ColorBarElement {
    fn element_id(&self) -> String {
        "colorbar".to_string()
    }

    /// The strip rect — the same one the renderer paints, offsets included, so
    /// the highlight box and the eight handles sit on the bar the user sees.
    /// Ticks and labels are outside it, as they are for an axis.
    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        visible_colorbar_rect(cfg).map(|(_, rect)| rect)
    }

    /// The strip with the outer half of its border, which is stroked centred
    /// on the strip's edge.
    fn contains(&self, cfg: &Config, _measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        visible_colorbar_rect(cfg).is_some_and(|(bar, rect)| {
            rect.expanded(bar.border_width.max(0.0) * 0.5)
                .contains(x, y)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }
}

impl Selectable for ColorBarAxisElement {
    fn element_id(&self) -> String {
        "colorbar_axis".to_string()
    }

    /// The line and ticks the renderer draws ([`MeasureText::drawn_parts`]);
    /// `None` when nothing is drawn or `measure` does not lay them out.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        drawn_union(cfg, measure, &Element::ColorBarAxis, 0.0).flatten()
    }

    /// Inside the selection box around the drawn line and ticks.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        drawn_box_hit(cfg, measure, &Element::ColorBarAxis, x, y).unwrap_or(false)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ColorBarLabelElement {
    fn element_id(&self) -> String {
        "colorbar_tick_labels".to_string()
    }

    /// The union of the labels the renderer draws
    /// ([`MeasureText::drawn_parts`]); `None` when it draws none or
    /// `measure` does not lay them out.
    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        drawn_union(cfg, measure, &Element::ColorBarLabel, 0.0).flatten()
    }

    /// Inside the selection box around the drawn labels.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        drawn_box_hit(cfg, measure, &Element::ColorBarLabel, x, y).unwrap_or(false)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ColorBarTitleElement {
    fn element_id(&self) -> String {
        "colorbar_title".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let (bar, rect) = visible_colorbar_rect(cfg)?;
        let title = &bar.axis.title_option;
        if !title.visible || title.text.segments.is_empty() {
            return None;
        }
        let m = measure.measure_rich(&title.text);
        Some(
            colorbar_title_placement(
                bar.side.clone(),
                &rect,
                &bar.axis,
                (title.offset_x, title.offset_y),
                m,
            )
            .rect(m),
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for LegendElement {
    /// The whole content document is measured as one rich text (`'\n'`
    /// segments break lines); shared placement expands it by `padding` and
    /// anchors it at the configured data-area corner.
    fn element_id(&self) -> String {
        "legend".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let lg = &cfg.legend;
        if !lg.visible || lg.content.segments.is_empty() {
            return None;
        }
        let da = cfg.data_area().ok()?;

        let m = measure.measure_rich(&lg.content);
        Some(legend_rect(
            &da,
            lg.corner,
            lg.padding,
            (lg.offset_x, lg.offset_y),
            m,
        ))
    }

    /// The box with the outer half of its border, which is stroked centred
    /// on the box edge.
    fn contains(&self, cfg: &Config, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        self.bounds(cfg, measure).is_some_and(|b| {
            b.expanded(crate::layout::LEGEND_BORDER_WIDTH * 0.5)
                .contains(x, y)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

// Overlay layer elements. Bounds come from the overlay's expanded panel; a
// drag is applied to that panel through the ordinary nudge path and only the
// moved fields are copied back into the overlay's own options.

fn overlay_id(layer: usize, inner: DecorationId) -> String {
    DecorationId::Overlay {
        layer,
        inner: Box::new(inner),
    }
    .element_id()
}

/// Copy the drag-owned fields of a panel's right axis back to the overlay.
/// `out_margin` is group geometry and never moves through these elements.
fn adopt_overlay_axis(target: &mut AxisOptions, panel: &AxisOptions) {
    target.line_offset = panel.line_offset;
    target.label_style.label_offset_x = panel.label_style.label_offset_x;
    target.label_style.label_offset_y = panel.label_style.label_offset_y;
    target.title_option.offset_x = panel.title_option.offset_x;
    target.title_option.offset_y = panel.title_option.offset_y;
}

fn drag_overlay_axis_element(
    cfg: &mut Config,
    layer: usize,
    element: Element,
    dx: f32,
    dy: f32,
) -> NudgeResult {
    let Some(mut panel) = cfg.overlay_panel(layer) else {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    };
    let result = panel.nudge(element, dx, dy);
    if result == NudgeResult::Moved {
        adopt_overlay_axis(&mut cfg.overlays[layer].y_axis, &panel.right_y);
    }
    result
}

impl Selectable for OverlayAxisElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::Axis(Side::Right))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let panel = cfg.overlay_panel(self.layer)?;
        AxisElement { side: Side::Right }.bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisElement {
    fn nudge_element(&self) -> Element {
        Element::Axis(Side::Right)
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), dx, dy)
    }
}

impl Selectable for OverlayAxisLabelElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisLabel(Side::Right))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let panel = cfg.overlay_panel(self.layer)?;
        AxisLabelElement { side: Side::Right }.bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisLabelElement {
    fn nudge_element(&self) -> Element {
        Element::AxisLabel(Side::Right)
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), dx, dy)
    }
}

impl Selectable for OverlayAxisTitleElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisTitle(Side::Right))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let panel = cfg.overlay_panel(self.layer)?;
        AxisTitleElement { side: Side::Right }.bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisTitleElement {
    fn nudge_element(&self) -> Element {
        Element::AxisTitle(Side::Right)
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), dx, dy)
    }
}

impl Selectable for OverlayLegendElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::Legend)
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        cfg.overlays.get(self.layer)?.legend.as_ref()?;
        let panel = cfg.overlay_panel(self.layer)?;
        LegendElement.bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayLegendElement {
    fn nudge_element(&self) -> Element {
        Element::Legend
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        if cfg
            .overlays
            .get(self.layer)
            .is_none_or(|overlay| overlay.legend.is_none())
        {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        }
        let Some(mut panel) = cfg.overlay_panel(self.layer) else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        let result = panel.nudge(Element::Legend, dx, dy);
        if result == NudgeResult::Moved
            && let Some(legend) = cfg.overlays[self.layer].legend.as_mut()
        {
            legend.offset_x = panel.legend.offset_x;
            legend.offset_y = panel.legend.offset_y;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default::default_config;
    use crate::layout::{ChartArea, Rect};
    use crate::text::{RichText, TextExtents, rich_segments_from_text};

    /// `element_id` gives every standard element a distinct, stable name —
    /// hosts key hover/context UI on these strings.
    #[test]
    fn element_ids_are_stable_and_distinct() {
        let ids = [
            DataAreaElement.element_id(),
            AxisElement { side: Side::Bottom }.element_id(),
            AxisLabelElement { side: Side::Left }.element_id(),
            AxisTitleElement { side: Side::Left }.element_id(),
            LegendElement.element_id(),
            ChartTitleElement.element_id(),
            ColorBarElement.element_id(),
            ColorBarAxisElement.element_id(),
            ColorBarLabelElement.element_id(),
            ColorBarTitleElement.element_id(),
        ];
        assert_eq!(
            ids,
            [
                "data_area",
                "axis_bottom",
                "tick_labels_left",
                "axis_title_left",
                "legend",
                "chart_title",
                "colorbar",
                "colorbar_axis",
                "colorbar_tick_labels",
                "colorbar_title"
            ]
        );
    }

    /// Deterministic stub mirroring the renderer's line splitting: 8 px per
    /// segment on the widest line, 10 up / 3 down per line, `'\n'` segments
    /// start a new 13 px line below the first baseline.
    struct FixedMeasure;
    impl MeasureText for FixedMeasure {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            let mut line_lens = vec![0usize];
            for seg in &rt.segments {
                if seg.text == '\n' {
                    line_lens.push(0);
                } else if seg.text != '\t' {
                    // '\t' is a column separator (zero-width here; the real
                    // engine aligns columns — irrelevant for these tests).
                    *line_lens.last_mut().unwrap() += 1;
                }
            }
            TextExtents {
                width: line_lens.iter().copied().max().unwrap_or(0) as f32 * 8.0,
                ascent: 10.0,
                descent: 3.0 + (line_lens.len() - 1) as f32 * 13.0,
            }
        }
    }

    fn cfg_800x600() -> Config {
        let mut cfg = default_config();
        cfg.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        });
        cfg
    }

    fn cfg_with_overlay() -> Config {
        let mut cfg = cfg_800x600();
        let mut overlay = crate::layer::OverlayLayer::default();
        overlay.y_axis.title_option.visible = true;
        overlay.y_axis.title_option.text = RichText::plain("Pressure", Color::BLACK, 12.0, "");
        overlay.y_axis.out_margin = 100.0;
        overlay.series = vec!["p".into()];
        cfg.overlays.push(overlay);
        cfg
    }

    fn ids_of(map: &HitMap) -> Vec<String> {
        (0..map.len())
            .map(|i| map.get(HitId(i)).unwrap().element_id())
            .collect()
    }

    // `Base` registers exactly the standard set; an active overlay registers
    // the shared elements first and its own elements last, and never the
    // base panel's Y axes. The annotation layer registers only the shared set.
    #[test]
    fn active_layer_registration_sets() {
        let mut cfg = cfg_with_overlay();
        assert_eq!(
            ids_of(&HitMap::for_active_layer(&cfg)),
            ids_of(&HitMap::standard_chart())
        );
        cfg.set_active_layer(LayerRef::Overlay(0));
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        assert_eq!(
            ids,
            [
                "data_area",
                "axis_top",
                "axis_bottom",
                "tick_labels_top",
                "tick_labels_bottom",
                "axis_title_top",
                "axis_title_bottom",
                "legend",
                "chart_title",
                "colorbar",
                "colorbar_axis",
                "colorbar_tick_labels",
                "colorbar_title",
                "overlay_0:axis_right",
                "overlay_0:tick_labels_right",
                "overlay_0:axis_title_right",
                "overlay_0:legend",
            ]
        );
        assert!(!ids.iter().any(|id| id == "axis_left" || id == "axis_right"));
        cfg.set_active_layer(LayerRef::Annotations);
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        assert_eq!(ids.len(), 13);
        assert!(!ids.iter().any(|id| id.starts_with("overlay_")));
        // An out-of-range index is the base set.
        cfg.active_layer = LayerRef::Overlay(3);
        assert_eq!(
            ids_of(&HitMap::for_active_layer(&cfg)),
            ids_of(&HitMap::standard_chart())
        );
    }

    /// The 800x600 parent and below it a subplot with its own axes, an
    /// overlay, and a Y axis linked to the parent's.
    fn cfg_with_subplot() -> Config {
        let mut cfg = cfg_800x600();
        let mut subplot = crate::layer::Subplot::default();
        subplot.config.chart_area = ChartArea(Rect {
            x: 0,
            y: 600,
            width: 800,
            height: 400,
        });
        subplot.config.bottom_x.min = 10.0;
        subplot.config.bottom_x.max = 20.0;
        subplot.config.overlays.push(crate::layer::OverlayLayer {
            series: vec!["level".into()],
            ..Default::default()
        });
        subplot.series = vec!["flow".into()];
        subplot.link_y = Some(crate::layer::CellRef::Parent);
        cfg.subplots.push(subplot);
        cfg.sync_subplot_links(None);
        cfg
    }

    // An active subplot registers exactly its own chart's set under
    // `subplot_<n>:` ids and none of the parent's elements; its overlay
    // registers the subplot's overlay set.
    #[test]
    fn a_subplot_registers_its_own_chart_set() {
        let mut cfg = cfg_with_subplot();
        cfg.set_active_layer(LayerRef::Subplot(0));
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        let own: Vec<String> = ids_of(&HitMap::standard_chart())
            .into_iter()
            .map(|id| format!("subplot_0:{id}"))
            .collect();
        assert_eq!(ids, own);
        cfg.set_active_layer(LayerRef::SubplotOverlay(0, 0));
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        assert!(ids.contains(&"subplot_0:overlay_0:axis_right".to_string()));
        assert!(!ids.contains(&"subplot_0:axis_left".to_string()));
        assert!(ids.iter().all(|id| id.starts_with("subplot_0:")));
    }

    // Hits resolve in the subplot's own area; drags and resizes use the
    // single-chart policies on the subplot's chart and land in
    // `subplots[0].config` only, keeping the link.
    #[test]
    fn subplot_elements_hit_drag_and_resize_their_own_chart() {
        let mut cfg = cfg_with_subplot();
        cfg.set_active_layer(LayerRef::Subplot(0));
        let map = HitMap::for_active_layer(&cfg);
        let da = cfg.subplot_config(0).unwrap().data_area().unwrap().0;
        assert!(da.y >= 600);
        let centre = (
            da.x as f32 + da.width as f32 / 2.0,
            da.y as f32 + da.height as f32 / 2.0,
        );
        let hit = map
            .hit_test(&cfg, &FixedMeasure, centre.0, centre.1)
            .unwrap();
        let element = map.get(hit).unwrap();
        assert_eq!(element.element_id(), "subplot_0:data_area");
        // The parent's data area does not answer while a subplot is active.
        let parent = cfg.data_area().unwrap().0;
        assert!(
            map.hit_test(
                &cfg,
                &FixedMeasure,
                parent.x as f32 + 40.0,
                parent.y as f32 + 40.0
            )
            .is_none()
        );

        let before = cfg.clone();
        let drag = element.as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, 10.0, -6.0), NudgeResult::Moved);
        let moved = cfg.subplot_config(0).unwrap().data_area().unwrap().0;
        assert_eq!((moved.x, moved.y), (da.x + 10, da.y - 6));
        assert_eq!(cfg.data_area(), before.data_area());
        assert_eq!(cfg.left_y, before.left_y);
        assert_eq!(cfg.subplots[0].link_y, before.subplots[0].link_y);
        assert_eq!(cfg.subplots[0].config.left_y.max, cfg.left_y.max);

        let resize = element.as_resizable().unwrap();
        assert_eq!(
            element
                .selection_box(&cfg, &FixedMeasure)
                .unwrap()
                .handles
                .len(),
            8
        );
        let handle = resize
            .resize_handles(&cfg, &FixedMeasure)
            .unwrap()
            .into_iter()
            .find(|(handle, _)| *handle == crate::resize::ResizeHandle::E)
            .unwrap();
        assert!(handle.1.x > moved.x as f32);
        assert_eq!(
            resize.resize_by(&mut cfg, crate::resize::ResizeHandle::E, -30.0, 0.0),
            NudgeResult::Moved
        );
        let resized = cfg.subplot_config(0).unwrap().data_area().unwrap().0;
        assert_eq!(resized.width, moved.width - 30);
        assert_eq!(cfg.margins(), before.margins());

        let axis = map.find("subplot_0:axis_bottom").unwrap();
        let drag = map.get(axis).unwrap().as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, 0.0, 5.0), NudgeResult::Moved);
        assert_eq!(cfg.subplots[0].config.bottom_x.line_offset, 5.0);
        assert_eq!(cfg.bottom_x, before.bottom_x);
    }

    #[test]
    fn overlay_legend_is_selectable_only_when_present() {
        let mut cfg = cfg_with_overlay();
        cfg.set_active_layer(LayerRef::Overlay(0));
        let mut legend = crate::default::default_legend();
        legend.visible = true;
        legend.content = RichText {
            segments: rich_segments_from_text("P"),
            ..legend.content
        };
        cfg.overlays[0].legend = Some(legend);
        let map = HitMap::for_active_layer(&cfg);
        let id = map.find("overlay_0:legend").unwrap();
        let bounds = map.get(id).unwrap().bounds(&cfg, &FixedMeasure).unwrap();
        let hit = map
            .hit_test(&cfg, &FixedMeasure, bounds.x + 1.0, bounds.y + 1.0)
            .unwrap();
        assert_eq!(hit, id, "the overlay legend is on top of the shared legend");
        let drag = map.get(id).unwrap().as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, -5.0, 6.0), NudgeResult::Moved);
        let moved = cfg.overlays[0].legend.as_ref().unwrap();
        assert_eq!((moved.offset_x, moved.offset_y), (-5.0, 6.0));
        assert_eq!((cfg.legend.offset_x, cfg.legend.offset_y), (0.0, 0.0));
        cfg.overlays[0].legend = None;
        assert!(map.get(id).unwrap().bounds(&cfg, &FixedMeasure).is_none());
        assert_eq!(
            drag.drag_by(&mut cfg, 1.0, 1.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
    }

    #[test]
    fn data_area_bounds_match_layout() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(b, RectF::from_rect(&da.0));
    }

    #[test]
    fn chart_title_bounds_centered_in_band() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("Title");

        let b = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        // 5 chars × 8 px = 40 wide, centered at x = 400.
        assert_eq!(b.width, 40.0);
        assert_eq!(b.height, 13.0);
        assert!((b.x - (400.0 - 20.0)).abs() < 1e-4);
        // Vertically inside the 32 px title band.
        assert!(b.y >= 0.0 && b.y + b.height <= cfg.chart_title.top_margin);
    }

    #[test]
    fn hidden_or_empty_title_has_no_bounds() {
        let mut cfg = cfg_800x600();
        // Empty text (default) → None.
        assert!(ChartTitleElement.bounds(&cfg, &FixedMeasure).is_none());
        // Filled but hidden → None.
        cfg.chart_title.text.segments = rich_segments_from_text("T");
        cfg.chart_title.visible = false;
        assert!(ChartTitleElement.bounds(&cfg, &FixedMeasure).is_none());
    }

    #[test]
    fn left_axis_title_bounds_swap_extents() {
        let mut cfg = cfg_800x600();
        cfg.left_y.title_option.text.segments = rich_segments_from_text("Volt"); // 4 chars

        let b = AxisTitleElement { side: Side::Left }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Rotated 90°: screen width = text height (13), screen height = text width (32).
        assert_eq!(b.width, 13.0);
        assert_eq!(b.height, 32.0);
    }

    #[test]
    fn selection_box_pads_bounds_with_blue() {
        let cfg = cfg_800x600();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let sb = DataAreaElement.selection_box(&cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.rect, b.expanded(SELECTION_PADDING));
        assert_eq!(sb.color, SELECTION_COLOR);
        assert_eq!(sb.stroke_width, SELECTION_STROKE_WIDTH);
    }

    #[test]
    fn contains_hits_inside_and_misses_outside() {
        let cfg = cfg_800x600();
        let b = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let (cx, cy) = (b.x + b.width * 0.5, b.y + b.height * 0.5);
        assert!(DataAreaElement.contains(&cfg, &FixedMeasure, cx, cy));
        assert!(!DataAreaElement.contains(&cfg, &FixedMeasure, b.x - 10.0, b.y - 10.0));
    }

    #[test]
    fn hitmap_topmost_wins_over_background() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("Title");
        let map = HitMap::standard_chart();

        // A point inside the chart title box must report the title, not the
        // (earlier-registered) bands behind it.
        let tb = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        let id = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                tb.x + tb.width * 0.5,
                tb.y + tb.height * 0.5,
            )
            .unwrap();
        let sb = map.selection_box(id, &cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.rect, tb.expanded(SELECTION_PADDING));

        // Center of the data area → data area element.
        let db = DataAreaElement.bounds(&cfg, &FixedMeasure).unwrap();
        let id2 = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                db.x + db.width * 0.5,
                db.y + db.height * 0.5,
            )
            .unwrap();
        assert_ne!(id, id2);

        // Far outside the chart → no hit.
        assert!(map.hit_test(&cfg, &FixedMeasure, 5000.0, 5000.0).is_none());
    }

    #[test]
    fn legend_drag_offset_moves_bounds() {
        use crate::legend::{LegendEntryKind, append_legend_entry, symbol_segments};
        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "a",
        );
        let before = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();

        // Drag through the trait-object route hosts use.
        let drag = LegendElement.as_draggable().expect("legend is draggable");
        assert_eq!(
            drag.drag_by(&mut cfg, 9.0, -5.0),
            crate::layout::NudgeResult::Moved
        );
        let after = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(after.x, before.x + 9.0);
        assert_eq!(after.y, before.y - 5.0);
    }

    #[test]
    fn data_area_selection_box_carries_resize_handles() {
        let cfg = cfg_800x600();
        let sb = DataAreaElement.selection_box(&cfg, &FixedMeasure).unwrap();
        assert_eq!(sb.handles.len(), 8);
        // Non-resizable elements carry none.
        let mut cfg = cfg;
        cfg.chart_title.text = RichText::plain("title", Color::BLACK, 14.0, "");
        let sb2 = ChartTitleElement
            .selection_box(&cfg, &FixedMeasure)
            .unwrap();
        assert!(sb2.handles.is_empty());
    }

    #[test]
    fn hidden_axis_labels_have_no_bounds() {
        let mut cfg = cfg_800x600();
        cfg.left_y.label_style.label_visible = false;
        assert!(
            AxisLabelElement { side: Side::Left }
                .bounds(&cfg, &FixedMeasure)
                .is_none()
        );
    }

    #[test]
    fn legend_bounds_mirror_renderer_box() {
        use crate::legend::{LegendCorner, LegendEntryKind, append_legend_entry, symbol_segments};

        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        cfg.legend.corner = LegendCorner::TopRight;
        // One-line content "— abc": 5 segments × 8 px wide, 13 px tall.
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );

        let da = cfg.data_area().unwrap();
        let lg = &cfg.legend;
        let b = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        let box_w = 5.0 * 8.0 + lg.padding * 2.0;
        let box_h = 13.0 + lg.padding * 2.0;
        assert_eq!(b.width, box_w);
        assert_eq!(b.height, box_h);
        // One-line content: wider than tall.
        assert!(b.width > b.height);
        assert_eq!(b.x, (da.x + da.width) as f32 - box_w - 6.0);
        assert_eq!(b.y, da.y as f32 + 6.0);

        // Invisible legend → no bounds.
        let mut hidden = cfg.clone();
        hidden.legend.visible = false;
        assert!(LegendElement.bounds(&hidden, &FixedMeasure).is_none());
    }

    #[test]
    fn legend_bounds_multiline_content_grows_taller() {
        use crate::legend::{LegendEntryKind, append_legend_entry, symbol_segments};

        let mut cfg = cfg_800x600();
        cfg.legend.visible = true;
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );
        let one_line = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();

        // Second append inserts an explicit '\n' → one more 13 px line; the
        // width stays the widest line's.
        append_legend_entry(
            &mut cfg.legend.content,
            symbol_segments(&LegendEntryKind::Line, crate::color::Color::BLACK),
            "abc",
        );
        let two_lines = LegendElement.bounds(&cfg, &FixedMeasure).unwrap();
        assert_eq!(two_lines.height, one_line.height + 13.0);
        assert_eq!(two_lines.width, one_line.width);
    }

    #[test]
    fn register_into_default_method_round_trips() {
        let mut cfg = cfg_800x600();
        cfg.chart_title.text.segments = rich_segments_from_text("T");
        let mut map = HitMap::new();
        let id = ChartTitleElement.register_into(&mut map);
        assert_eq!(map.len(), 1);
        let tb = ChartTitleElement.bounds(&cfg, &FixedMeasure).unwrap();
        let hit = map
            .hit_test(&cfg, &FixedMeasure, tb.x + 1.0, tb.y + 1.0)
            .unwrap();
        assert_eq!(hit, id);
    }

    // ── Colourbar ──────────────────────────────────────────────────────────

    fn cfg_with_colorbar(side: Side) -> Config {
        let mut cfg = cfg_800x600();
        let mut bar = crate::default::default_colorbar_options();
        bar.side = side;
        cfg.colorbar = Some(bar);
        cfg
    }

    /// The bar is selectable like every other piece of chrome, and its bounds
    /// are the strip the renderer paints — not the band, and not the labels.
    #[test]
    fn the_colorbar_is_selectable_and_its_bounds_are_the_strip() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let cfg = cfg_with_colorbar(side.clone());
            let da = cfg.data_area().expect("data area");
            let bar = cfg.colorbar.as_ref().expect("colourbar");
            let expected = colorbar_rect(&cfg.chart_area, &da, cfg.chart_title.top_margin, bar);
            assert_eq!(
                ColorBarElement.bounds(&cfg, &FixedMeasure),
                Some(expected),
                "{side:?}"
            );

            // And it is reachable through the standard hit map.
            let map = HitMap::standard_chart();
            let id = map
                .hit_test(
                    &cfg,
                    &FixedMeasure,
                    expected.x + expected.width * 0.5,
                    expected.y + expected.height * 0.5,
                )
                .expect("the strip is hit-testable");
            assert_eq!(map.get(id).expect("element").element_id(), "colorbar");
        }
    }

    /// No colourbar, or a hidden one, is not selectable — the same rule the
    /// legend and the titles follow when they are not drawn.
    #[test]
    fn an_absent_or_hidden_colorbar_is_not_selectable() {
        let cfg = cfg_800x600();
        assert_eq!(ColorBarElement.bounds(&cfg, &FixedMeasure), None);

        let mut hidden = cfg_with_colorbar(Side::Right);
        hidden.colorbar.as_mut().expect("colourbar").visible = false;
        assert_eq!(ColorBarElement.bounds(&hidden, &FixedMeasure), None);

        // A degenerate strip has nothing to select either.
        let mut degenerate = cfg_with_colorbar(Side::Right);
        degenerate.colorbar.as_mut().expect("colourbar").length_frac = 0.0;
        assert_eq!(ColorBarElement.bounds(&degenerate, &FixedMeasure), None);
    }

    /// Resizable elements get the eight handles automatically. Before this the
    /// data area was the only one; the bar is the second.
    #[test]
    fn the_colorbar_selection_box_carries_resize_handles() {
        let cfg = cfg_with_colorbar(Side::Right);
        let box_ = ColorBarElement
            .selection_box(&cfg, &FixedMeasure)
            .expect("selection box");
        assert_eq!(box_.handles.len(), 8);
        assert_eq!(
            box_.rect,
            ColorBarElement
                .bounds(&cfg, &FixedMeasure)
                .expect("bounds")
                .expanded(SELECTION_PADDING)
        );

        // Non-resizable chrome still has none, so the handles mean something.
        let title = ChartTitleElement.selection_box(&cfg, &FixedMeasure);
        assert!(title.is_none_or(|b| b.handles.is_empty()));
    }

    #[test]
    fn hidden_colorbar_details_have_no_bounds() {
        let mut cfg = cfg_with_colorbar(Side::Right);
        cfg.colorbar.as_mut().unwrap().visible = false;
        assert!(ColorBarAxisElement.bounds(&cfg, &FixedMeasure).is_none());
        assert!(ColorBarLabelElement.bounds(&cfg, &FixedMeasure).is_none());
        assert!(ColorBarTitleElement.bounds(&cfg, &FixedMeasure).is_none());
    }
}
