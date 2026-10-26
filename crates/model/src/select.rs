//! Selection policy — `Selectable`, the geometry sinks and the chart
//! elements.
//!
//! Every element has one placement function, its [`Selectable::emit`], which
//! hands the parts it draws to a [`Sink`]. What selection *does* is fixed by
//! the three sinks of this module, which every element shares through the
//! blanket [`SelectionGeometry`] trait: the bounds are the union of the
//! emitted parts, the hit region is what the selection outlines, and the
//! selection box is built from the same parts. No element can override them,
//! so a drawn element, its hit region and its box cannot disagree. The
//! renderer draws the box (`axis_render::draw_selection_box`); the model only
//! states the policy.
//!
//! The elements the model can place itself (titles, the legend, the colourbar
//! strip and title, the data area) emit through [`emit_element`], which the
//! renderer's raster also draws with. Tick labels and axis strokes need tick
//! generation, formatting and pruning, which belong to the renderer, so their
//! parts come from the renderer's own emit through
//! [`MeasureText::emit`]; a measure that does not lay them out leaves them
//! without bounds.

use crate::color::Color;
use crate::config::{AxisOptions, ChartTitleOptions, Config};
use crate::decoration::DecorationId;
use crate::drag::{DragConfig, Draggable};
use crate::layer::LayerRef;
use crate::layout::{
    Element, LEGEND_BORDER_WIDTH, NudgeReject, NudgeResult, RectF, Side, axis_title_placement,
    chart_title_placement_in, colorbar_title_placement, legend_rect, visible_colorbar_rect,
};
use crate::resize::{Resizable, ResizeHandle};
use crate::sink::{Border, DrawnPart, Parts, PlacedText, Sink, Slot, TextRef, union};
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
/// Least half width of the box a stroke is outlined and grabbed with where
/// an element hits on its parts (see [`segment_grab_half_width`]).
pub const STROKE_GRAB_HALF_WIDTH: f32 = 3.0;

/// What a selection highlight outlines.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SelectionOutline {
    /// The box [`SelectionBox::rect`].
    #[default]
    Rect,
    /// Each drawn part of the element, where it is drawn, in place of `rect`,
    /// which still bounds them: a label block as its box, a stroke as the
    /// thin oriented box around it, a triangle as itself. Polar tick labels
    /// ring the plot or line the ray and an arrow is a diagonal, so one box
    /// around them would cover the chart. The parts are the ones the
    /// selection sink saw; nothing is laid out twice.
    Parts(Vec<DrawnPart>),
}

/// The blue box drawn around a selected element, in the chart frame. Built
/// only by [`SelectionGeometry::selection_box`], from the element's emitted
/// parts; the renderer's raster pass and host UI read it through its
/// accessors and map it into their own frame and scale.
#[derive(Debug, Clone, PartialEq)]
pub struct SelectionBox {
    rect: RectF,
    color: Color,
    stroke_width: f32,
    handles: Vec<RectF>,
    outline: SelectionOutline,
}

impl SelectionBox {
    /// Box rect in chart-surface pixels (already padding-expanded).
    pub fn rect(&self) -> RectF {
        self.rect
    }

    pub fn color(&self) -> Color {
        self.color
    }

    pub fn stroke_width(&self) -> f32 {
        self.stroke_width
    }

    /// Resize handle squares (empty for non-resizable elements). Drawn as
    /// filled squares on top of the box outline.
    pub fn handles(&self) -> &[RectF] {
        &self.handles
    }

    /// What the highlight outlines: `rect`, or each drawn part.
    pub fn outline(&self) -> &SelectionOutline {
        &self.outline
    }

    /// This box inside `area`: its rectangle intersected with it, for an
    /// element its canvas clips. The handles and the part outlines are
    /// kept.
    pub fn clipped_to(mut self, area: RectF) -> Self {
        let r = self.rect;
        let (x0, y0) = (r.x.max(area.x), r.y.max(area.y));
        let x1 = (r.x + r.width).min(area.x + area.width);
        let y1 = (r.y + r.height).min(area.y + area.height);
        self.rect = RectF {
            x: x0,
            y: y0,
            width: (x1 - x0).max(0.0),
            height: (y1 - y0).max(0.0),
        };
        self
    }
}

/// An object the user can select on the chart.
///
/// Implementations provide [`Self::emit`], the element's one placement, and
/// declare their capabilities. Its bounds, hit test and selection box are
/// the sinks of [`SelectionGeometry`], which every `Selectable` gets and
/// none can override.
pub trait Selectable<C: DragConfig = Config> {
    /// Place every part this element draws under `cfg` into `sink`, in the
    /// chart frame, and return `true`; return `false` when `measure` cannot
    /// lay the element out (tick labels and axes under a measure that does
    /// not place them). An element that is hidden, empty or infeasible
    /// places nothing and returns `true`.
    fn emit(&self, cfg: &C, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool;

    /// Whether the selection outlines each drawn part instead of one box
    /// around them, and the element hits on its parts instead of inside the
    /// box: polar tick labels and the polar ray, annotations.
    fn outlines_parts(&self, _cfg: &C) -> bool {
        false
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

/// The bounds, hit test and selection box of every [`Selectable`]: the three
/// geometry sinks run over the element's [`Selectable::emit`].
///
/// Blanket-implemented for every `Selectable`, so no element can supply a
/// second placement for any of the three.
pub trait SelectionGeometry<C: DragConfig = Config> {
    /// The union of the parts the element draws under `cfg`, or `None` when
    /// it draws none or `measure` cannot lay it out.
    fn bounds(&self, cfg: &C, measure: &dyn MeasureText) -> Option<RectF>;

    /// Whether `(x, y)` is inside what the selection outlines: the box
    /// [`SELECTION_PADDING`] px around the bounds, or, for an element that
    /// [`Selectable::outlines_parts`], the [`part_outline`] of one of its
    /// parts.
    fn contains(&self, cfg: &C, measure: &dyn MeasureText, x: f32, y: f32) -> bool;

    /// The selection policy: a blue box [`SELECTION_PADDING`] px outside the
    /// bounds, stroked [`SELECTION_STROKE_WIDTH`] px in [`SELECTION_COLOR`],
    /// with the element's resize handles, outlining the box or each part.
    fn selection_box(&self, cfg: &C, measure: &dyn MeasureText) -> Option<SelectionBox>;
}

/// The extent of a drawn part for the bounds: what is drawn, without grab
/// slack.
fn part_extent(part: &DrawnPart) -> RectF {
    part.extent()
}

/// The shape a per-part selection draws around one drawn part, in the
/// part's frame, and the region the part is grabbed in: the one geometry of
/// the hit test, the renderer's highlight and a host's outline. A text block
/// or rectangle is its box grown by [`SELECTION_PADDING`]; a stroke is the
/// box [`segment_grab_half_width`] either side of its line and the padding
/// beyond its ends; a triangle is its [`padded_triangle`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PartOutline {
    Quad([[f32; 2]; 4]),
    Triangle([[f32; 2]; 3]),
}

impl PartOutline {
    /// The corners in drawing order; the polygon closes on the first.
    pub fn corners(&self) -> &[[f32; 2]] {
        match self {
            PartOutline::Quad(c) => c,
            PartOutline::Triangle(c) => c,
        }
    }

    /// Whether `(x, y)` is inside the outline, its edge included.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        let c = self.corners();
        let side =
            |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]) * (y - a[1]) - (b[1] - a[1]) * (x - a[0]);
        let (mut positive, mut negative) = (false, false);
        for i in 0..c.len() {
            let d = side(c[i], c[(i + 1) % c.len()]);
            positive |= d > 0.0;
            negative |= d < 0.0;
        }
        !(positive && negative)
    }
}

/// The [`PartOutline`] of `part`.
pub fn part_outline(part: &DrawnPart) -> PartOutline {
    match *part {
        DrawnPart::Text(r) | DrawnPart::Rect(r) => {
            let r = r.expanded(SELECTION_PADDING);
            PartOutline::Quad([
                [r.x, r.y],
                [r.x + r.width, r.y],
                [r.x + r.width, r.y + r.height],
                [r.x, r.y + r.height],
            ])
        }
        DrawnPart::Segment { a, b, width } => {
            let d = [b[0] - a[0], b[1] - a[1]];
            let len = d[0].hypot(d[1]);
            let (dir, n) = if len > 0.0 {
                ([d[0] / len, d[1] / len], [-d[1] / len, d[0] / len])
            } else {
                ([1.0, 0.0], [0.0, 1.0])
            };
            let half = segment_grab_half_width(width);
            let along = SELECTION_PADDING;
            let p = |s: f32, t: f32| [a[0] + dir[0] * s + n[0] * t, a[1] + dir[1] * s + n[1] * t];
            PartOutline::Quad([
                p(-along, -half),
                p(len + along, -half),
                p(len + along, half),
                p(-along, half),
            ])
        }
        DrawnPart::Triangle(t) => PartOutline::Triangle(padded_triangle(t)),
    }
}

/// Whether `(x, y)` is inside the outline of a drawn part.
fn part_contains(part: &DrawnPart, x: f32, y: f32) -> bool {
    part_outline(part).contains(x, y)
}

/// Half the width of the box a stroke `width` across is outlined and
/// grabbed with: half the stroke plus [`SELECTION_PADDING`], at least
/// [`STROKE_GRAB_HALF_WIDTH`].
pub fn segment_grab_half_width(width: f32) -> f32 {
    (width * 0.5 + SELECTION_PADDING).max(STROKE_GRAB_HALF_WIDTH)
}

/// The triangle `t` with every edge moved [`SELECTION_PADDING`] px outward:
/// the outline of a triangle part. A degenerate triangle is returned as it
/// is.
pub fn padded_triangle(t: [[f32; 2]; 3]) -> [[f32; 2]; 3] {
    let cx = (t[0][0] + t[1][0] + t[2][0]) / 3.0;
    let cy = (t[0][1] + t[1][1] + t[2][1]) / 3.0;
    // The unit normal of the edge `a`..`b` that points away from the centroid.
    let outward = |a: [f32; 2], b: [f32; 2]| -> Option<[f32; 2]> {
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len <= 0.0 {
            return None;
        }
        let n = [-(b[1] - a[1]) / len, (b[0] - a[0]) / len];
        let to_centroid = (cx - a[0]) * n[0] + (cy - a[1]) * n[1];
        Some(if to_centroid > 0.0 { [-n[0], -n[1]] } else { n })
    };
    let mut out = t;
    for i in 0..3 {
        let (prev, v, next) = (t[(i + 2) % 3], t[i], t[(i + 1) % 3]);
        let (Some(n1), Some(n2)) = (outward(prev, v), outward(v, next)) else {
            return t;
        };
        let denom = 1.0 + n1[0] * n2[0] + n1[1] * n2[1];
        if denom <= 1e-6 {
            return t;
        }
        let k = SELECTION_PADDING / denom;
        out[i] = [v[0] + k * (n1[0] + n2[0]), v[1] + k * (n1[1] + n2[1])];
    }
    out
}

/// The bounds sink: `Some(None)` when the element draws nothing, `None`
/// when `measure` cannot lay it out.
fn drawn_union<C: DragConfig>(
    element: &(impl Selectable<C> + ?Sized),
    cfg: &C,
    measure: &dyn MeasureText,
) -> Option<Option<RectF>> {
    let mut bounds: Option<RectF> = None;
    let laid_out = element.emit(
        cfg,
        measure,
        &mut Parts(&mut |part| {
            let r = part_extent(&part);
            bounds = Some(bounds.map_or(r, |u| union(u, r)));
        }),
    );
    laid_out.then_some(bounds)
}

impl<C: DragConfig, T: Selectable<C> + ?Sized> SelectionGeometry<C> for T {
    fn bounds(&self, cfg: &C, measure: &dyn MeasureText) -> Option<RectF> {
        drawn_union(self, cfg, measure).flatten()
    }

    fn contains(&self, cfg: &C, measure: &dyn MeasureText, x: f32, y: f32) -> bool {
        if self.outlines_parts(cfg) {
            let mut hit = false;
            self.emit(
                cfg,
                measure,
                &mut Parts(&mut |part| hit |= part_contains(&part, x, y)),
            );
            hit
        } else {
            self.bounds(cfg, measure)
                .is_some_and(|b| b.expanded(SELECTION_PADDING).contains(x, y))
        }
    }

    fn selection_box(&self, cfg: &C, measure: &dyn MeasureText) -> Option<SelectionBox> {
        let outline = if self.outlines_parts(cfg) {
            let mut parts = Vec::new();
            if !self.emit(cfg, measure, &mut Parts(&mut |part| parts.push(part))) {
                return None;
            }
            SelectionOutline::Parts(parts)
        } else {
            SelectionOutline::Rect
        };
        let rect = match &outline {
            // The parts are in hand: their union is the bounds.
            SelectionOutline::Parts(parts) => parts
                .iter()
                .map(DrawnPart::extent)
                .reduce(crate::sink::union)?,
            SelectionOutline::Rect => self.bounds(cfg, measure)?,
        }
        .expanded(SELECTION_PADDING);
        let mut handles = Vec::new();
        if let Some(resizable) = self.as_resizable() {
            resizable.resize_handles(cfg, measure, &mut |_, square| handles.push(square));
        }
        Some(SelectionBox {
            rect,
            color: SELECTION_COLOR,
            stroke_width: SELECTION_STROKE_WIDTH,
            handles,
            outline,
        })
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
    /// element's [`SelectionGeometry::selection_box`] policy.
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
            // Every field of the edited chart is named: a new one is stored
            // from the edit unless it is one `subplot_config` overrides.
            let Config {
                chart_area,
                top_x,
                bottom_x,
                left_y,
                right_y,
                chart_title,
                grid,
                legend,
                picked_points,
                picked_data,
                colorbar,
                draw_style,
                polar,
                overlays,
                subplots,
                // `subplot_config` resets these two; the stored ones stay.
                active_layer: _,
                annotations: _,
                bar_groups,
                line_offsets,
            } = chart;
            *stored = Config {
                chart_area,
                top_x,
                bottom_x,
                left_y,
                right_y,
                chart_title,
                grid,
                legend,
                picked_points,
                picked_data,
                colorbar,
                draw_style,
                polar,
                overlays,
                subplots,
                active_layer: stored.active_layer,
                annotations: std::mem::take(&mut stored.annotations),
                bar_groups,
                line_offsets,
            };
        }
        result
    }
}

impl Selectable for SubplotElement {
    fn element_id(&self) -> String {
        format!("subplot_{}:{}", self.subplot, self.inner.element_id())
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        match cfg.subplot_config(self.subplot) {
            Some(chart) => self.inner.emit(&chart, measure, sink),
            None => true,
        }
    }

    fn outlines_parts(&self, cfg: &Config) -> bool {
        cfg.subplot_config(self.subplot)
            .is_some_and(|chart| self.inner.outlines_parts(&chart))
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

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| drag.drag_by(chart, measure, dx, dy))
    }

    fn drag_from(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        at: [f32; 2],
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(drag) = self.inner.as_draggable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        // A subplot's chart keeps document coordinates, so the pointer
        // passes through unchanged.
        self.edit(cfg, |chart| drag.drag_from(chart, measure, at, dx, dy))
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
        measure: &dyn MeasureText,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let Some(resize) = self.inner.as_resizable() else {
            return NudgeResult::Rejected(NudgeReject::OutOfBounds);
        };
        self.edit(cfg, |chart| {
            resize.resize_by(chart, measure, handle, dx, dy)
        })
    }

    fn resize_handles(
        &self,
        cfg: &Config,
        measure: &dyn MeasureText,
        each: &mut dyn FnMut(ResizeHandle, RectF),
    ) {
        if let (Some(resize), Some(chart)) =
            (self.inner.as_resizable(), cfg.subplot_config(self.subplot))
        {
            resize.resize_handles(&chart, measure, each);
        }
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

/// Run `f` on the config the renderer draws the base layer of `cfg` with: a
/// single-layer document as it is; a layered one as its expanded base panel
/// (the expansion's own copy of the SSOT configuration, borrowed here), or
/// without its layers when it does not expand at this size, as the
/// renderer's panel expansion does.
pub fn on_drawn_panel<R>(cfg: &Config, f: impl FnOnce(&Config) -> R) -> R {
    if cfg.overlays.is_empty() && cfg.subplots.is_empty() {
        return f(cfg);
    }
    match cfg.expand() {
        Ok(expansion) => match expansion.panel(LayerRef::Base) {
            Some(panel) => f(&panel.config),
            None => f(cfg),
        },
        Err(_) => {
            let mut panel = cfg.clone();
            panel.overlays.clear();
            panel.subplots.clear();
            panel.active_layer = LayerRef::Base;
            f(&panel)
        }
    }
}

// The model's own placements: the elements it can lay out from the layout
// functions and a text measure. The renderer draws them through the same
// function.

/// Place decoration `element` of `cfg` into `sink`, in `cfg`'s chart frame,
/// and return whether it could be laid out: the one placement of the chart
/// title, the axis titles, the legend, the colourbar strip and its title and
/// the data area, which the renderer's raster draws with too. Tick labels,
/// axis strokes and the colourbar's axis and labels are placed by `measure`
/// ([`MeasureText::emit`]), which lays them out with the renderer's tick
/// plan, formatting and pruning or returns `false`.
///
/// `cfg` is drawn as it is: a caller with a layered document passes the
/// panel the layer is drawn with (the elements of this module do).
pub fn emit_element(
    cfg: &Config,
    element: &Element,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) -> bool {
    match element {
        Element::ChartTitle => emit_chart_title(cfg, measure, sink),
        Element::AxisTitle(side) => emit_axis_title(cfg, side, measure, sink),
        Element::Legend => emit_legend(cfg, measure, sink),
        Element::ColorBar => emit_colorbar_strip(cfg, sink),
        Element::ColorBarTitle => emit_colorbar_title(cfg, measure, sink),
        Element::DataArea => emit_data_area(cfg, sink),
        Element::AxisLabel(_)
        | Element::Axis(_)
        | Element::ColorBarAxis
        | Element::ColorBarLabel => {
            return measure.emit(cfg, element, sink);
        }
        // Resize targets: nothing of their own is drawn.
        Element::DataAreaEdge(_) | Element::ColorBarEdge(_) => {}
    }
    true
}

fn emit_chart_title(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let ct = &cfg.chart_title;
    emit_chart_title_in(
        ct,
        RectF::from_rect(&cfg.chart_area),
        ct.top_margin,
        measure,
        sink,
    );
}

/// The one placement of a chart title: `title` centred across `chart_area`
/// and in the band `top_margin` high at its top, moved by the title's
/// offsets. A Cartesian chart passes its chart area and the title's own
/// band; a chart that lays out its own title band (the radial chart) passes
/// its canvas and that band. Places nothing for a hidden or empty title.
pub fn emit_chart_title_in(
    title: &ChartTitleOptions,
    chart_area: RectF,
    top_margin: f32,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) {
    let ct = title;
    if !ct.visible || ct.text.segments.is_empty() {
        return;
    }
    let m = measure.measure_rich(&ct.text);
    let placement = chart_title_placement_in(chart_area, top_margin, (ct.offset_x, ct.offset_y), m);
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&ct.text),
            origin: placement.origin,
            extents: m,
            rotation: None,
        },
    );
}

/// The axis title on `side`. A polar chart draws no top or right axis, so
/// their titles are not placed either.
fn emit_axis_title(cfg: &Config, side: &Side, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    if cfg.polar.is_some() && matches!(side, Side::Top | Side::Right) {
        return;
    }
    let axis = axis_of(cfg, side);
    let to = &axis.title_option;
    if !to.visible || to.text.segments.is_empty() {
        return;
    }
    let Ok(da) = cfg.data_area() else {
        return;
    };
    let m = measure.measure_rich(&to.text);
    let placement = axis_title_placement(
        side.clone(),
        &cfg.chart_area,
        &da,
        cfg.title_band(side),
        (to.offset_x, to.offset_y),
        m,
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&to.text),
            origin: placement.origin,
            extents: m,
            rotation: (placement.rotation_degrees != 0.0)
                .then_some((placement.rotation_degrees, placement.rotation_center)),
        },
    );
}

/// The legend box in one corner of the data area: the box with its fill and
/// border, then the content, whose first baseline sits `ascent` below the
/// padded top-left corner.
fn emit_legend(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let lg = &cfg.legend;
    if !lg.visible || lg.content.segments.is_empty() {
        return;
    }
    let Ok(da) = cfg.data_area() else {
        return;
    };
    let m = measure.measure_rich(&lg.content);
    let rect = legend_rect(&da, lg.corner, lg.padding, (lg.offset_x, lg.offset_y), m);
    sink.rect(
        Slot::Above,
        rect,
        Some(lg.bg_color),
        Some(Border {
            color: lg.border_color,
            width: LEGEND_BORDER_WIDTH,
            tag: "legend_box",
        }),
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&lg.content),
            origin: (rect.x + lg.padding, rect.y + lg.padding + m.ascent),
            extents: m,
            rotation: None,
        },
    );
}

/// The colourbar strip: the ramp and its border.
fn emit_colorbar_strip(cfg: &Config, sink: &mut dyn Sink) {
    let Some((bar, rect)) = visible_colorbar_rect(cfg) else {
        return;
    };
    sink.ramp(Slot::Above, rect, bar);
    if bar.border_width > 0.0 {
        sink.rect(
            Slot::Above,
            rect,
            None,
            Some(Border {
                color: bar.border_color,
                width: bar.border_width,
                tag: "colorbar_strip",
            }),
        );
    }
}

/// The colourbar title, anchored on the painted strip so shortening,
/// aligning, dragging or resizing the bar moves the title with it.
fn emit_colorbar_title(cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) {
    let Some((bar, rect)) = visible_colorbar_rect(cfg) else {
        return;
    };
    let to = &bar.axis.title_option;
    if !to.visible || to.text.segments.is_empty() {
        return;
    }
    let m = measure.measure_rich(&to.text);
    let placement = colorbar_title_placement(
        bar.side.clone(),
        &rect,
        &bar.axis,
        (to.offset_x, to.offset_y),
        m,
    );
    sink.text(
        Slot::Above,
        &PlacedText {
            text: TextRef::Rich(&to.text),
            origin: placement.origin,
            extents: m,
            rotation: (placement.rotation_degrees != 0.0)
                .then_some((placement.rotation_degrees, placement.rotation_center)),
        },
    );
}

/// The data area: a rectangle that paints nothing of its own.
fn emit_data_area(cfg: &Config, sink: &mut dyn Sink) {
    if let Ok(da) = cfg.data_area() {
        sink.rect(Slot::Above, RectF::from_rect(&da.0), None, None);
    }
}

impl Selectable for AxisElement {
    fn element_id(&self) -> String {
        format!("axis_{}", side_str(&self.side))
    }

    /// The line and ticks the renderer draws ([`MeasureText::emit`]): a
    /// polar chart's angle axis is the outer ring, which is the data boundary
    /// and not selectable, and its radius ray hits on each stroke.
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        if cfg.polar.is_some() && self.side != Side::Left {
            return true;
        }
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::Axis(self.side.clone()), measure, sink)
        })
    }

    fn outlines_parts(&self, cfg: &Config) -> bool {
        cfg.polar.is_some()
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisLabelElement {
    fn element_id(&self) -> String {
        format!("tick_labels_{}", side_str(&self.side))
    }

    /// The labels the renderer draws ([`MeasureText::emit`]).
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::AxisLabel(self.side.clone()), measure, sink)
        })
    }

    /// Polar tick labels ring the plot (angle) or line the ray (radius):
    /// each label is outlined and hit where it is drawn.
    fn outlines_parts(&self, cfg: &Config) -> bool {
        cfg.polar.is_some()
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
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::AxisTitle(self.side.clone()), measure, sink)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ChartTitleElement {
    fn element_id(&self) -> String {
        "chart_title".to_string()
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::ChartTitle, measure, sink)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for DataAreaElement {
    fn element_id(&self) -> String {
        "data_area".to_string()
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::DataArea, measure, sink)
        })
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

    /// The strip the renderer paints, offsets included, so the highlight box
    /// and the eight handles sit on the bar the user sees. Ticks and labels
    /// are outside it, as they are for an axis.
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::ColorBar, measure, sink)
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

    /// The line and ticks the renderer draws ([`MeasureText::emit`]).
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::ColorBarAxis, measure, sink)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ColorBarLabelElement {
    fn element_id(&self) -> String {
        "colorbar_tick_labels".to_string()
    }

    /// The labels the renderer draws ([`MeasureText::emit`]).
    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::ColorBarLabel, measure, sink)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ColorBarTitleElement {
    fn element_id(&self) -> String {
        "colorbar_title".to_string()
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::ColorBarTitle, measure, sink)
        })
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

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        on_drawn_panel(cfg, |panel| {
            emit_element(panel, &Element::Legend, measure, sink)
        })
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

// Overlay layer elements. Parts come from the overlay's expanded panel; a
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
    measure: &dyn MeasureText,
    dx: f32,
    dy: f32,
) -> NudgeResult {
    let Some(mut panel) = cfg.overlay_panel(layer) else {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    };
    let result = panel.nudge(element, measure, dx, dy);
    if result == NudgeResult::Moved {
        adopt_overlay_axis(&mut cfg.overlays[layer].y_axis, &panel.right_y);
    }
    result
}

/// Emit `inner` on the overlay panel of `layer`; nothing while the document
/// has no such panel.
fn emit_on_overlay_panel(
    cfg: &Config,
    layer: usize,
    inner: &dyn Selectable,
    measure: &dyn MeasureText,
    sink: &mut dyn Sink,
) -> bool {
    match cfg.overlay_panel(layer) {
        Some(panel) => inner.emit(&panel, measure, sink),
        None => true,
    }
}

impl Selectable for OverlayAxisElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::Axis(Side::Right))
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        emit_on_overlay_panel(
            cfg,
            self.layer,
            &AxisElement { side: Side::Right },
            measure,
            sink,
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisElement {
    fn nudge_element(&self) -> Element {
        Element::Axis(Side::Right)
    }

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), measure, dx, dy)
    }
}

impl Selectable for OverlayAxisLabelElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisLabel(Side::Right))
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        emit_on_overlay_panel(
            cfg,
            self.layer,
            &AxisLabelElement { side: Side::Right },
            measure,
            sink,
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisLabelElement {
    fn nudge_element(&self) -> Element {
        Element::AxisLabel(Side::Right)
    }

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), measure, dx, dy)
    }
}

impl Selectable for OverlayAxisTitleElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisTitle(Side::Right))
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        emit_on_overlay_panel(
            cfg,
            self.layer,
            &AxisTitleElement { side: Side::Right },
            measure,
            sink,
        )
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisTitleElement {
    fn nudge_element(&self) -> Element {
        Element::AxisTitle(Side::Right)
    }

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), measure, dx, dy)
    }
}

impl Selectable for OverlayLegendElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::Legend)
    }

    fn emit(&self, cfg: &Config, measure: &dyn MeasureText, sink: &mut dyn Sink) -> bool {
        if cfg
            .overlays
            .get(self.layer)
            .is_none_or(|overlay| overlay.legend.is_none())
        {
            return true;
        }
        emit_on_overlay_panel(cfg, self.layer, &LegendElement, measure, sink)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayLegendElement {
    fn nudge_element(&self) -> Element {
        Element::Legend
    }

    fn drag_by(
        &self,
        cfg: &mut Config,
        measure: &dyn MeasureText,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
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
        let result = panel.nudge(Element::Legend, measure, dx, dy);
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
pub(crate) mod tests {
    #[test]
    fn part_outlines_are_what_is_grabbed() {
        // A 1 px horizontal stroke from (10, 10) to (50, 10): grabbed within
        // 3 px either side and 2 px beyond either end, as its box is drawn.
        let (a, b) = ([10.0, 10.0], [50.0, 10.0]);
        assert_eq!(segment_grab_half_width(1.0), STROKE_GRAB_HALF_WIDTH);
        assert_eq!(segment_grab_half_width(6.0), 3.0 + SELECTION_PADDING);
        let stroke = |a, b, width| part_outline(&DrawnPart::Segment { a, b, width });
        let thin = stroke(a, b, 1.0);
        assert!(thin.contains(30.0, 12.9));
        assert!(!thin.contains(30.0, 13.1));
        assert!(thin.contains(8.1, 10.0));
        assert!(!thin.contains(7.9, 10.0));
        assert!(thin.contains(51.9, 10.0));
        assert!(!thin.contains(52.1, 10.0));
        assert_eq!(
            thin,
            PartOutline::Quad([[8.0, 7.0], [52.0, 7.0], [52.0, 13.0], [8.0, 13.0]])
        );
        // A degenerate stroke is a dot grabbed within the same box.
        let dot = stroke(a, a, 1.0);
        assert!(dot.contains(12.0, 10.0));
        assert!(!dot.contains(13.5, 10.0));
        // A rectangle grows by the padding on every side.
        let rect = RectF {
            x: 10.0,
            y: 10.0,
            width: 20.0,
            height: 5.0,
        };
        assert_eq!(
            part_outline(&DrawnPart::Rect(rect)),
            PartOutline::Quad([[8.0, 8.0], [32.0, 8.0], [32.0, 17.0], [8.0, 17.0]])
        );

        // A right triangle grows 2 px outward on every edge: the point 1 px
        // outside the bottom edge is inside the padded triangle, 3 px is not.
        let t = [[0.0, 0.0], [40.0, 0.0], [0.0, 30.0]];
        let padded = part_outline(&DrawnPart::Triangle(t));
        assert!(padded.contains(20.0, -1.0));
        assert!(!padded.contains(20.0, -3.0));
        let PartOutline::Triangle(corners) = padded else {
            panic!("a triangle outlines a triangle");
        };
        assert!((corners[0][0] + SELECTION_PADDING).abs() < 1e-5);
        assert!((corners[0][1] + SELECTION_PADDING).abs() < 1e-5);
        for v in t {
            assert!(padded.contains(v[0], v[1]));
        }
        assert_eq!(padded_triangle([a, a, b]), [a, a, b]);
    }

    use super::*;
    use crate::default::default_config;
    use crate::layout::{
        ChartArea, LABEL_GAP, Rect, axis_anchor, axis_cross, axis_offset, colorbar_rect,
        label_origin, point_on_rect_side,
    };
    use crate::sink::StrokeStyle;
    use crate::text::{RichText, TextExtents, rich_segments_from_text};

    /// A test measure that lays out what only a renderer can place: one tick
    /// label per `Element::AxisLabel` / `Element::ColorBarLabel` and one
    /// stroke per `Element::Axis` / `Element::ColorBarAxis`, where the
    /// renderer draws them (the label at the middle of its side, just past
    /// the ticks, moved by the axis' `line_offset` and label offsets; the
    /// axis line on the data-area edge, moved by its `line_offset`). Polar
    /// charts get four angle labels on the outer ring, one radius label
    /// beside the ray and the ray itself. Text is measured as
    /// [`FixedMeasure`] measures it.
    pub(crate) struct LaysOut;

    /// The extents of every placed tick label: three 8 px glyphs.
    const LABEL_EXTENTS: TextExtents = TextExtents {
        width: 24.0,
        ascent: 10.0,
        descent: 3.0,
    };

    fn place_label(sink: &mut dyn Sink, origin: (f32, f32)) {
        sink.text(
            Slot::Above,
            &PlacedText {
                text: TextRef::Plain {
                    text: "0.5",
                    family: "",
                    size: 12.0,
                    color: Color::BLACK,
                },
                origin,
                extents: LABEL_EXTENTS,
                rotation: None,
            },
        );
    }

    /// A label whose box is centred on `center`.
    fn place_label_at(sink: &mut dyn Sink, center: [f32; 2]) {
        place_label(
            sink,
            (
                center[0] - LABEL_EXTENTS.width * 0.5,
                center[1] + (LABEL_EXTENTS.ascent - LABEL_EXTENTS.descent) * 0.5,
            ),
        );
    }

    fn place_stroke(sink: &mut dyn Sink, a: [f32; 2], b: [f32; 2]) {
        sink.stroke(
            Slot::Above,
            a,
            b,
            &StrokeStyle {
                width: 1.0,
                color: Color::BLACK,
                dash: &[],
                tag: "test_axis",
            },
        );
    }

    fn lay_out_polar(cfg: &Config, element: &Element, sink: &mut dyn Sink) {
        let (Some(polar), Some(g)) = (cfg.polar.as_ref(), cfg.polar_geometry()) else {
            return;
        };
        let c = g.center;
        match element {
            Element::AxisLabel(Side::Bottom) if cfg.bottom_x.label_style.label_visible => {
                let reach = g.radius
                    + LABEL_GAP
                    + LABEL_EXTENTS.width * 0.5
                    + crate::polar::PolarOptions::angle_label_offset(&cfg.bottom_x.label_style);
                for k in 0..4 {
                    let a = polar.zero_angle_degrees.to_radians()
                        + f64::from(k) * 0.5 * std::f64::consts::PI;
                    let dir = [a.cos() as f32, -(a.sin() as f32)];
                    place_label_at(sink, [c[0] + dir[0] * reach, c[1] + dir[1] * reach]);
                }
            }
            Element::AxisLabel(Side::Left) if cfg.left_y.label_style.label_visible => {
                let (d, s) = (polar.ray_direction(), polar.ray_side());
                let o = polar.radius_label_offset(&cfg.left_y.label_style);
                let across = LABEL_GAP + LABEL_EXTENTS.width * 0.5;
                let along = g.radius * 0.5;
                place_label_at(
                    sink,
                    [
                        c[0] + d[0] * along + s[0] * across + o[0],
                        c[1] + d[1] * along + s[1] * across + o[1],
                    ],
                );
            }
            Element::Axis(Side::Left) => {
                let d = polar.ray_direction();
                place_stroke(
                    sink,
                    [c[0] + d[0] * g.inner_radius, c[1] + d[1] * g.inner_radius],
                    [c[0] + d[0] * g.radius, c[1] + d[1] * g.radius],
                );
            }
            _ => {}
        }
    }

    impl MeasureText for LaysOut {
        fn measure_rich(&self, rt: &RichText) -> TextExtents {
            FixedMeasure.measure_rich(rt)
        }

        fn emit(&self, cfg: &Config, element: &Element, sink: &mut dyn Sink) -> bool {
            match element {
                Element::AxisLabel(side) | Element::Axis(side) => {
                    if cfg.polar.is_some() {
                        lay_out_polar(cfg, element, sink);
                        return true;
                    }
                    let Ok(da) = cfg.data_area() else {
                        return true;
                    };
                    let axis = axis_of(cfg, side);
                    let plot = RectF::from_rect(&da);
                    let o = axis_offset(side.clone(), axis.line_offset);
                    if let Element::Axis(_) = element {
                        // The line crosses where `cross_at` puts it; the
                        // labels stay at the edge.
                        let (a, b) = axis_anchor(side.clone(), &plot, axis_cross(cfg, side));
                        if axis.line_visible {
                            place_stroke(sink, [a.0 + o.0, a.1 + o.1], [b.0 + o.0, b.1 + o.1]);
                        }
                    } else if axis.label_style.label_visible {
                        let (a, b) = axis_anchor(side.clone(), &plot, None);
                        let mid = ((a.0 + b.0) * 0.5 + o.0, (a.1 + b.1) * 0.5 + o.1);
                        let offset = (
                            axis.label_style.label_offset_x,
                            axis.label_style.label_offset_y,
                        );
                        place_label(
                            sink,
                            label_origin(
                                side.clone(),
                                mid,
                                axis.major_tick_length,
                                offset,
                                LABEL_EXTENTS,
                            ),
                        );
                    }
                    true
                }
                Element::ColorBarAxis | Element::ColorBarLabel => {
                    let Some((bar, rect)) = visible_colorbar_rect(cfg) else {
                        return true;
                    };
                    let side = bar.side.clone();
                    let a = point_on_rect_side(0.0, &side, &rect);
                    let b = point_on_rect_side(1.0, &side, &rect);
                    let o = axis_offset(side.clone(), bar.axis.line_offset);
                    if let Element::ColorBarAxis = element {
                        place_stroke(sink, [a.0 + o.0, a.1 + o.1], [b.0 + o.0, b.1 + o.1]);
                    } else if bar.axis.label_style.label_visible {
                        let mid = ((a.0 + b.0) * 0.5 + o.0, (a.1 + b.1) * 0.5 + o.1);
                        let offset = (
                            bar.axis.label_style.label_offset_x,
                            bar.axis.label_style.label_offset_y,
                        );
                        place_label(
                            sink,
                            label_origin(
                                side,
                                mid,
                                bar.axis.major_tick_length,
                                offset,
                                LABEL_EXTENTS,
                            ),
                        );
                    }
                    true
                }
                _ => false,
            }
        }
    }

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
    pub(crate) struct FixedMeasure;
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
        assert_eq!(
            drag.drag_by(&mut cfg, &FixedMeasure, 10.0, -6.0),
            NudgeResult::Moved
        );
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
                .handles()
                .len(),
            8
        );
        let mut east = None;
        resize.resize_handles(&cfg, &FixedMeasure, &mut |handle, square| {
            if handle == crate::resize::ResizeHandle::E {
                east = Some(square);
            }
        });
        assert!(east.unwrap().x > moved.x as f32);
        assert_eq!(
            resize.resize_by(
                &mut cfg,
                &FixedMeasure,
                crate::resize::ResizeHandle::E,
                -30.0,
                0.0
            ),
            NudgeResult::Moved
        );
        let resized = cfg.subplot_config(0).unwrap().data_area().unwrap().0;
        assert_eq!(resized.width, moved.width - 30);
        assert_eq!(cfg.margins(), before.margins());

        let axis = map.find("subplot_0:axis_bottom").unwrap();
        let drag = map.get(axis).unwrap().as_draggable().unwrap();
        assert_eq!(
            drag.drag_by(&mut cfg, &FixedMeasure, 0.0, 5.0),
            NudgeResult::Moved
        );
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
        assert_eq!(
            drag.drag_by(&mut cfg, &FixedMeasure, -5.0, 6.0),
            NudgeResult::Moved
        );
        let moved = cfg.overlays[0].legend.as_ref().unwrap();
        assert_eq!((moved.offset_x, moved.offset_y), (-5.0, 6.0));
        assert_eq!((cfg.legend.offset_x, cfg.legend.offset_y), (0.0, 0.0));
        cfg.overlays[0].legend = None;
        assert!(map.get(id).unwrap().bounds(&cfg, &FixedMeasure).is_none());
        assert_eq!(
            drag.drag_by(&mut cfg, &FixedMeasure, 1.0, 1.0),
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
        assert_eq!(sb.rect(), b.expanded(SELECTION_PADDING));
        assert_eq!(sb.color(), SELECTION_COLOR);
        assert_eq!(sb.stroke_width(), SELECTION_STROKE_WIDTH);
        assert_eq!(sb.outline(), &SelectionOutline::Rect);
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
        assert_eq!(sb.rect(), tb.expanded(SELECTION_PADDING));

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
            drag.drag_by(&mut cfg, &FixedMeasure, 9.0, -5.0),
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
        assert_eq!(sb.handles().len(), 8);
        // Non-resizable elements carry none.
        let mut cfg = cfg;
        cfg.chart_title.text = RichText::plain("title", Color::BLACK, 14.0, "");
        let sb2 = ChartTitleElement
            .selection_box(&cfg, &FixedMeasure)
            .unwrap();
        assert!(sb2.handles().is_empty());
    }

    // Tick labels are placed by the measure: a measure that lays them out
    // gives drawn labels bounds and hidden ones none, and a measure that
    // cannot place them leaves them without bounds either way.
    #[test]
    fn hidden_axis_labels_have_no_bounds() {
        let mut cfg = cfg_800x600();
        let labels = AxisLabelElement { side: Side::Left };
        assert!(labels.bounds(&cfg, &LaysOut).is_some());
        assert!(labels.bounds(&cfg, &FixedMeasure).is_none());
        cfg.left_y.label_style.label_visible = false;
        assert!(labels.bounds(&cfg, &LaysOut).is_none());
        assert!(labels.bounds(&cfg, &FixedMeasure).is_none());
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
        assert_eq!(box_.handles().len(), 8);
        assert_eq!(
            box_.rect(),
            ColorBarElement
                .bounds(&cfg, &FixedMeasure)
                .expect("bounds")
                .expanded(SELECTION_PADDING)
        );

        // Non-resizable chrome still has none, so the handles mean something.
        let title = ChartTitleElement.selection_box(&cfg, &FixedMeasure);
        assert!(title.is_none_or(|b| b.handles().is_empty()));
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
