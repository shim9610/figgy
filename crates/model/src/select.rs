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
//! the [`MeasureText`] contract. Tick labels intentionally use a representative
//! strip instead of reproducing renderer-owned tick generation, formatting,
//! pruning, or the exact union of rendered labels.

use crate::color::Color;
use crate::config::{AxisOptions, Config, TickVisibility};
use crate::decoration::DecorationId;
use crate::drag::{DragConfig, Draggable};
use crate::expand::OverlayPlacement;
use crate::layer::LayerRef;
use crate::layout::{
    Element, NudgeReject, NudgeResult, RectF, Side, TitleBand, axis_offset, axis_title_placement,
    axis_visibility_rect, chart_title_placement, colorbar_rect, colorbar_title_placement,
    label_origin, label_rect, legend_rect, point_on_rect_side, rect_axis_visibility_rect,
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
}

/// An object the user can select on the chart.
///
/// Implementations provide [`Self::bounds`] only. Selection behavior lives in
/// the default methods so it is identical across elements; override them only
/// to change the policy itself.
pub trait Selectable<C: DragConfig = Config> {
    /// Interaction bounds of this element under `cfg`, or `None` when the
    /// element is hidden, empty, or the layout is infeasible. Text-bearing
    /// elements use `measure`; tick-label elements remain representative
    /// strips rather than exact rendered-label unions.
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
        })
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
    /// reverse-order hit test puts them on top: for `Overlay(i)` its Y axis,
    /// that axis' label band and title, and its per-layer legend
    /// (`DecorationId::Overlay { layer: i, .. }` ids, `overlay_<i>:…`). A
    /// layer drawn in an area of its own also registers that area's data
    /// area and its copies of the two X axes, between the shared elements
    /// and its Y axis.
    /// The inactive layers' elements are not registered at all, so a click
    /// on them passes through to the active layer; the base panel's Y axes
    /// are therefore absent while an overlay or the annotation layer is
    /// active. For `Annotations` the map holds the shared elements only:
    /// the annotation items live in their own renderer-measured
    /// `HitMap<AnnotationLayer>`, and the pointer route (the renderer's
    /// `ChartView::hit_test_active_layer`) tests them on top of this map
    /// while that layer is active and skips them on every other layer.
    ///
    /// The set depends only on the active layer, the overlay count and the
    /// active overlay's placement, so a host rebuilds the map when any of
    /// them changes; `HitId`s from a previous map are not valid in the new
    /// one.
    pub fn for_active_layer(cfg: &Config) -> Self {
        let layer = cfg.active_layer();
        if layer == LayerRef::Base {
            return Self::standard_chart();
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
        if let LayerRef::Overlay(index) = layer
            && let Some(placement) = cfg.overlay_placement(index)
        {
            // A layer in an area of its own also carries that area's data
            // area and its copies of the X axes, before its own Y axis.
            let sides = if placement == OverlayPlacement::OnParent {
                vec![placement.y_side()]
            } else {
                map.register(OverlayDataAreaElement::new(index));
                vec![Side::Top, Side::Bottom, placement.y_side()]
            };
            for side in sides.iter().cloned() {
                map.register(OverlayAxisElement { layer: index, side });
            }
            for side in sides.iter().cloned() {
                map.register(OverlayAxisLabelElement { layer: index, side });
            }
            for side in sides {
                map.register(OverlayAxisTitleElement { layer: index, side });
            }
            map.register(OverlayLegendElement { layer: index });
        }
        map
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

/// One axis of overlay layer `layer` — line plus tick band.
///
/// `side` is where the layer draws it: its own Y axis on the side of its
/// placement (`OverlayPlacement::y_side`), or `Top` / `Bottom` for the copy
/// of the parent's X axis drawn in the layer's own area. Overlay elements
/// measure against the internal panel that draws them (`Config::overlay_panel`
/// of the layer, or of its area's first layer for the X axes) and write a
/// drag back into `overlays[layer].y_axis` for the Y axis, or into the
/// parent's `top_x` / `bottom_x` for an X axis, which exists once.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisElement {
    pub layer: usize,
    pub side: Side,
}

/// The tick-label band of one axis of overlay layer `layer`; see
/// [`OverlayAxisElement`] for `side`.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisLabelElement {
    pub layer: usize,
    pub side: Side,
}

/// The title of one axis of overlay layer `layer`; see
/// [`OverlayAxisElement`] for `side`.
#[derive(Debug, Clone, PartialEq)]
pub struct OverlayAxisTitleElement {
    pub layer: usize,
    pub side: Side,
}

/// The data area of an overlay layer drawn in an area of its own. Dragging
/// it moves the area — every layer of that area together — and its eight
/// handles move the area's edges; the parent and the other areas stay put.
/// The area is whole pixels, so the sub-pixel part of each pointer delta is
/// carried to the next one instead of being dropped.
#[derive(Debug)]
pub struct OverlayDataAreaElement {
    pub layer: usize,
    carry: [std::sync::atomic::AtomicU32; 2],
}

impl OverlayDataAreaElement {
    pub fn new(layer: usize) -> Self {
        Self {
            layer,
            carry: [
                std::sync::atomic::AtomicU32::new(0.0_f32.to_bits()),
                std::sync::atomic::AtomicU32::new(0.0_f32.to_bits()),
            ],
        }
    }

    /// Add `delta` to the carried remainder of `axis` and take its whole
    /// pixels.
    fn whole_pixels(&self, axis: usize, delta: f32) -> i64 {
        use std::sync::atomic::Ordering;
        let total = f32::from_bits(self.carry[axis].load(Ordering::Relaxed)) + delta;
        let whole = total.trunc();
        self.carry[axis].store((total - whole).to_bits(), Ordering::Relaxed);
        whole as i64
    }

    fn reset_carry(&self) {
        use std::sync::atomic::Ordering;
        for carry in &self.carry {
            carry.store(0.0_f32.to_bits(), Ordering::Relaxed);
        }
    }
}

impl PartialEq for OverlayDataAreaElement {
    fn eq(&self, other: &Self) -> bool {
        self.layer == other.layer
    }
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

fn representative_label_extents(
    axis: &AxisOptions,
    measure: &dyn MeasureText,
) -> crate::text::TextExtents {
    let ls = &axis.label_style;
    let sample_text = match &ls.format {
        crate::format::LabelFormat::Timestamp(_) => "0000-00-00 00:00:00.000".to_string(),
        _ => {
            let digits = (ls.significant_digits.max(1) as usize) + 2;
            "0".repeat(digits)
        }
    };
    let sample = crate::text::RichText {
        segments: crate::text::rich_segments_from_text(&sample_text),
        color: ls.color,
        font_size: ls.font_size,
        font: ls.label_font.clone(),
        align: crate::text::TextAlign::Left,
    };
    measure.measure_rich(&sample)
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

impl Selectable for AxisElement {
    fn element_id(&self) -> String {
        format!("axis_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        let axis = axis_of(cfg, &self.side);
        if !axis.line_visible && matches!(axis.tick, TickVisibility::None) {
            return None;
        }
        let da = cfg.data_area().ok()?;

        Some(axis_visibility_rect(self.side.clone(), &da, axis))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisLabelElement {
    /// Approximate interaction strip for representative label glyphs. It uses
    /// the renderer's placement formula and a decimal/timestamp sample, but
    /// does not reproduce the renderer-owned tick set, formatting, pruning,
    /// or exact rendered-label union.
    fn element_id(&self) -> String {
        format!("tick_labels_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let axis = axis_of(cfg, &self.side);
        let ls = &axis.label_style;
        if !ls.visible || !ls.label_visible {
            return None;
        }
        let da = cfg.data_area().ok()?;
        // Representative label extents at the label font/size. Digits share
        // one height; width approximates a `significant_digits`-long number
        // (+2 for a sign / decimal point).
        let m = representative_label_extents(axis, measure);

        let (dax, day) = (da.x as f32, da.y as f32);
        let (daw, dah) = (da.width as f32, da.height as f32);
        let tick_position = match self.side {
            Side::Top => (dax, day),
            Side::Bottom => (dax, day + dah),
            Side::Left => (dax, day),
            Side::Right => (dax + daw, day),
        };
        let origin = label_origin(
            self.side.clone(),
            tick_position,
            axis.major_tick_length,
            (ls.label_offset_x, ls.label_offset_y),
            m,
        );
        let mut strip = label_rect(origin, m);
        match self.side {
            Side::Top | Side::Bottom => {
                strip.x = dax;
                strip.width = daw;
            }
            Side::Left | Side::Right => {
                strip.y = day;
                strip.height = dah;
            }
        }
        let (dx, dy) = axis_offset(self.side.clone(), axis.line_offset);
        Some(strip.translated(dx, dy))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for AxisTitleElement {
    fn element_id(&self) -> String {
        format!("axis_title_{}", side_str(&self.side))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
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

    fn bounds(&self, cfg: &Config, _measure: &dyn MeasureText) -> Option<RectF> {
        let (bar, rect) = visible_colorbar_rect(cfg)?;
        let axis = &bar.axis;
        if !axis.line_visible && matches!(axis.tick, TickVisibility::None) {
            return None;
        }
        Some(rect_axis_visibility_rect(bar.side.clone(), &rect, axis))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Selectable for ColorBarLabelElement {
    fn element_id(&self) -> String {
        "colorbar_tick_labels".to_string()
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        let (bar, rect) = visible_colorbar_rect(cfg)?;
        let axis = &bar.axis;
        let ls = &axis.label_style;
        if !ls.visible || !ls.label_visible {
            return None;
        }

        let m = representative_label_extents(axis, measure);
        let tick_position = point_on_rect_side(0.0, &bar.side, &rect);
        let origin = label_origin(
            bar.side.clone(),
            tick_position,
            axis.major_tick_length,
            (ls.label_offset_x, ls.label_offset_y),
            m,
        );
        let mut strip = label_rect(origin, m);
        match bar.side {
            Side::Top | Side::Bottom => {
                strip.x = rect.x;
                strip.width = rect.width;
            }
            Side::Left | Side::Right => {
                strip.y = rect.y;
                strip.height = rect.height;
            }
        }
        let (dx, dy) = axis_offset(bar.side.clone(), axis.line_offset);
        Some(strip.translated(dx, dy))
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

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

// Overlay layer elements. Bounds come from the expanded panel that draws
// them; a drag is applied to that panel through the ordinary nudge path and
// only the moved fields are copied back into the document: the layer's own
// axis, or the parent's X axis, which exists once.

fn overlay_id(layer: usize, inner: DecorationId) -> String {
    DecorationId::Overlay {
        layer,
        inner: Box::new(inner),
    }
    .element_id()
}

/// The panel that draws `side` of overlay `layer`: its own, except for the X
/// axes of a later layer of an area, which that area's first layer draws.
fn overlay_chrome_panel(cfg: &Config, layer: usize, side: &Side) -> Option<Config> {
    match (cfg.overlay_placement(layer)?, side) {
        (OverlayPlacement::OnArea { lead }, Side::Top | Side::Bottom) => cfg.overlay_panel(lead),
        _ => cfg.overlay_panel(layer),
    }
}

/// `true` when overlay `layer` draws (or carries the copy of) `side`.
fn overlay_has_side(cfg: &Config, layer: usize, side: &Side) -> bool {
    match cfg.overlay_placement(layer) {
        None => false,
        Some(OverlayPlacement::OnParent) => *side == Side::Right,
        Some(placement) => matches!(side, Side::Top | Side::Bottom) || *side == placement.y_side(),
    }
}

/// Copy the drag-owned fields of a panel axis back to the document axis.
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
    let side = match &element {
        Element::Axis(side) | Element::AxisLabel(side) | Element::AxisTitle(side) => side.clone(),
        _ => return NudgeResult::Rejected(NudgeReject::OutOfBounds),
    };
    if !overlay_has_side(cfg, layer, &side) {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    }
    let Some(mut panel) = overlay_chrome_panel(cfg, layer, &side) else {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    };
    let result = panel.nudge(element, dx, dy);
    if result == NudgeResult::Moved {
        let moved = crate::layout::axis_options_mut(&mut panel, &side).clone();
        let target = match side {
            Side::Top => &mut cfg.top_x,
            Side::Bottom => &mut cfg.bottom_x,
            Side::Left | Side::Right => &mut cfg.overlays[layer].y_axis,
        };
        adopt_overlay_axis(target, &moved);
    }
    result
}

impl Selectable for OverlayAxisElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::Axis(self.side.clone()))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        if !overlay_has_side(cfg, self.layer, &self.side) {
            return None;
        }
        let panel = overlay_chrome_panel(cfg, self.layer, &self.side)?;
        AxisElement {
            side: self.side.clone(),
        }
        .bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisElement {
    fn nudge_element(&self) -> Element {
        Element::Axis(self.side.clone())
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), dx, dy)
    }
}

impl Selectable for OverlayAxisLabelElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisLabel(self.side.clone()))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        if !overlay_has_side(cfg, self.layer, &self.side) {
            return None;
        }
        let panel = overlay_chrome_panel(cfg, self.layer, &self.side)?;
        AxisLabelElement {
            side: self.side.clone(),
        }
        .bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisLabelElement {
    fn nudge_element(&self) -> Element {
        Element::AxisLabel(self.side.clone())
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        drag_overlay_axis_element(cfg, self.layer, self.nudge_element(), dx, dy)
    }
}

impl Selectable for OverlayAxisTitleElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::AxisTitle(self.side.clone()))
    }

    fn bounds(&self, cfg: &Config, measure: &dyn MeasureText) -> Option<RectF> {
        if !overlay_has_side(cfg, self.layer, &self.side) {
            return None;
        }
        let panel = overlay_chrome_panel(cfg, self.layer, &self.side)?;
        AxisTitleElement {
            side: self.side.clone(),
        }
        .bounds(&panel, measure)
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }
}

impl Draggable for OverlayAxisTitleElement {
    fn nudge_element(&self) -> Element {
        Element::AxisTitle(self.side.clone())
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

/// Move the edges of overlay `layer`'s area — and of every layer sharing it —
/// by whole pixels: `left`/`top` move the near edges, `right`/`bottom` the far
/// ones (a pure move passes the same delta for both). Rejected atomically
/// when an edge would leave the coordinate space or the area would lose its
/// data area.
fn move_overlay_area(cfg: &mut Config, layer: usize, edges: [i64; 4]) -> NudgeResult {
    if !cfg.overlay_is_detached(layer) {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    }
    if edges == [0; 4] {
        return NudgeResult::Moved;
    }
    let old = cfg.overlay_area(layer);
    let [left, top, right, bottom] = edges;
    let x0 = i64::from(old.x) + left;
    let y0 = i64::from(old.y) + top;
    let x1 = i64::from(old.x) + i64::from(old.width) + right;
    let y1 = i64::from(old.y) + i64::from(old.height) + bottom;
    let max = i64::from(u32::MAX);
    if x0 < 0 || y0 < 0 || x1 > max || y1 > max || x1 <= x0 || y1 <= y0 {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    }
    let moved = crate::layout::ChartArea(crate::layout::Rect {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    });
    let mut trial = cfg.clone();
    for overlay in &mut trial.overlays {
        if overlay.chart_area.as_ref() == Some(&old) {
            overlay.chart_area = Some(moved.clone());
        }
    }
    if trial.overlay_data_area(layer).is_err() {
        return NudgeResult::Rejected(NudgeReject::OutOfBounds);
    }
    *cfg = trial;
    NudgeResult::Moved
}

impl Selectable for OverlayDataAreaElement {
    fn element_id(&self) -> String {
        overlay_id(self.layer, DecorationId::DataArea)
    }

    fn bounds(&self, cfg: &Config, _: &dyn MeasureText) -> Option<RectF> {
        if !cfg.overlay_is_detached(self.layer) {
            return None;
        }
        let area = cfg.overlay_data_area(self.layer).ok()?;
        Some(RectF::from_rect(&area.0))
    }

    fn as_draggable(&self) -> Option<&dyn Draggable> {
        Some(self)
    }

    fn as_resizable(&self) -> Option<&dyn Resizable> {
        Some(self)
    }
}

impl Draggable for OverlayDataAreaElement {
    fn nudge_element(&self) -> Element {
        Element::DataArea
    }

    fn drag_by(&self, cfg: &mut Config, dx: f32, dy: f32) -> NudgeResult {
        let dx = self.whole_pixels(0, dx);
        let dy = self.whole_pixels(1, dy);
        let result = move_overlay_area(cfg, self.layer, [dx, dy, dx, dy]);
        if result != NudgeResult::Moved {
            self.reset_carry();
        }
        result
    }
}

impl Resizable for OverlayDataAreaElement {
    /// The same compass mapping as the parent's data area; the edges are
    /// applied to the layer's area by [`Self::resize_by`].
    fn resize_targets(
        &self,
        handle: crate::resize::ResizeHandle,
    ) -> (Option<Element>, Option<Element>) {
        DataAreaElement.resize_targets(handle)
    }

    fn resize_by(
        &self,
        cfg: &mut Config,
        handle: crate::resize::ResizeHandle,
        dx: f32,
        dy: f32,
    ) -> NudgeResult {
        let (horizontal, vertical) = self.resize_targets(handle);
        let dx = self.whole_pixels(0, dx);
        let dy = self.whole_pixels(1, dy);
        let mut edges = [0_i64; 4];
        match horizontal {
            Some(Element::DataAreaEdge(Side::Left)) => edges[0] = dx,
            Some(Element::DataAreaEdge(Side::Right)) => edges[2] = dx,
            _ => {}
        }
        match vertical {
            Some(Element::DataAreaEdge(Side::Top)) => edges[1] = dy,
            Some(Element::DataAreaEdge(Side::Bottom)) => edges[3] = dy,
            _ => {}
        }
        let result = move_overlay_area(cfg, self.layer, edges);
        if result != NudgeResult::Moved {
            self.reset_carry();
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

    // The overlay's elements sit where the overlay panel draws them: on the
    // right side of the group data area. A hit there resolves to the overlay
    // axis, and a drag moves `overlays[0].y_axis` only.
    #[test]
    fn overlay_elements_hit_and_drag_the_overlay_axis_only() {
        let mut cfg = cfg_with_overlay();
        cfg.set_active_layer(LayerRef::Overlay(0));
        let map = HitMap::for_active_layer(&cfg);
        let da = cfg.data_area().unwrap().0;
        let panel = cfg.overlay_panel(0).unwrap();
        assert_eq!(panel.data_area().unwrap().0, da);
        let right_edge = (da.x + da.width) as f32;
        let mid_y = da.y as f32 + da.height as f32 / 2.0;
        // Ticks point inside by default: the axis band starts at the edge.
        let hit = map
            .hit_test(&cfg, &FixedMeasure, right_edge - 1.0, mid_y)
            .expect("the overlay axis band is on the right edge");
        assert_eq!(map.get(hit).unwrap().element_id(), "overlay_0:axis_right");
        let labels = map.find("overlay_0:tick_labels_right").unwrap();
        let label_bounds = map
            .get(labels)
            .unwrap()
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        assert!(label_bounds.x > right_edge, "{label_bounds:?}");
        let title = map.find("overlay_0:axis_title_right").unwrap();
        let title_bounds = map.get(title).unwrap().bounds(&cfg, &FixedMeasure).unwrap();
        assert!(title_bounds.x > label_bounds.x, "{title_bounds:?}");
        // No per-layer legend: no bounds, no hit.
        let legend = map.find("overlay_0:legend").unwrap();
        assert!(
            map.get(legend)
                .unwrap()
                .bounds(&cfg, &FixedMeasure)
                .is_none()
        );

        let before = cfg.clone();
        let axis = map.get(hit).unwrap().as_draggable().unwrap();
        assert_eq!(axis.drag_by(&mut cfg, -12.0, 40.0), NudgeResult::Moved);
        assert_eq!(cfg.overlays[0].y_axis.line_offset, -12.0);
        assert_eq!(cfg.right_y, before.right_y);
        assert_eq!(cfg.data_area().unwrap().0, da);
        let labels_drag = map.get(labels).unwrap().as_draggable().unwrap();
        assert_eq!(labels_drag.drag_by(&mut cfg, 3.0, -4.0), NudgeResult::Moved);
        assert_eq!(cfg.overlays[0].y_axis.label_style.label_offset_x, 3.0);
        assert_eq!(cfg.overlays[0].y_axis.label_style.label_offset_y, -4.0);
        let title_drag = map.get(title).unwrap().as_draggable().unwrap();
        assert_eq!(title_drag.drag_by(&mut cfg, 0.0, 10.0), NudgeResult::Moved);
        assert_ne!(
            cfg.overlays[0].y_axis.title_option,
            before.overlays[0].y_axis.title_option
        );
        assert_eq!(cfg.right_y, before.right_y);
        assert_eq!(cfg.left_y, before.left_y);
        assert_eq!(
            cfg.overlays[0].y_axis.out_margin,
            before.overlays[0].y_axis.out_margin
        );
        // A move that would leave the chart area is refused atomically.
        let snapshot = cfg.clone();
        assert_eq!(
            axis.drag_by(&mut cfg, 10_000.0, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, snapshot);
    }

    /// The 800x600 parent in the top half of a 800x1000 document and a
    /// subplot in the bottom 400 rows with two layers.
    fn cfg_with_subplot() -> Config {
        let mut cfg = cfg_800x600();
        let area = Some(crate::layout::ChartArea(crate::layout::Rect {
            x: 0,
            y: 600,
            width: 800,
            height: 400,
        }));
        let mut lead = crate::layer::OverlayLayer::default();
        lead.y_axis.title_option.visible = true;
        lead.y_axis.title_option.text = RichText::plain("Flow", Color::BLACK, 12.0, "");
        lead.y_axis.out_margin = 80.0;
        lead.series = vec!["flow".into()];
        lead.chart_area = area.clone();
        cfg.overlays.push(lead);
        let mut twin = crate::layer::OverlayLayer::default();
        twin.series = vec!["level".into()];
        twin.chart_area = area;
        cfg.overlays.push(twin);
        cfg
    }

    // A subplot's first layer registers its area's data area, the X axis
    // copies and its Y axis on the left; a later layer of the area the same
    // with its Y axis on the right.
    #[test]
    fn subplot_layers_register_their_area() {
        let mut cfg = cfg_with_subplot();
        cfg.set_active_layer(LayerRef::Overlay(0));
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        assert_eq!(
            &ids[13..],
            [
                "overlay_0:data_area",
                "overlay_0:axis_top",
                "overlay_0:axis_bottom",
                "overlay_0:axis_left",
                "overlay_0:tick_labels_top",
                "overlay_0:tick_labels_bottom",
                "overlay_0:tick_labels_left",
                "overlay_0:axis_title_top",
                "overlay_0:axis_title_bottom",
                "overlay_0:axis_title_left",
                "overlay_0:legend",
            ]
        );
        cfg.set_active_layer(LayerRef::Overlay(1));
        let ids = ids_of(&HitMap::for_active_layer(&cfg));
        assert_eq!(ids[13], "overlay_1:data_area");
        assert!(ids.contains(&"overlay_1:axis_right".to_string()));
        assert!(!ids.contains(&"overlay_1:axis_left".to_string()));
        assert!(ids.contains(&"overlay_1:axis_bottom".to_string()));
    }

    // Hits in the subplot resolve to the subplot's elements, measured in its
    // area; the X axis copies write the parent's X axes, the left axis the
    // layer's own axis.
    #[test]
    fn subplot_elements_hit_and_drag_their_owners() {
        let mut cfg = cfg_with_subplot();
        cfg.set_active_layer(LayerRef::Overlay(0));
        let map = HitMap::for_active_layer(&cfg);
        let da = cfg.overlay_data_area(0).unwrap().0;
        assert!(da.y >= 600);
        let mid_x = da.x as f32 + da.width as f32 / 2.0;
        let mid_y = da.y as f32 + da.height as f32 / 2.0;
        let hit = map.hit_test(&cfg, &FixedMeasure, mid_x, mid_y).unwrap();
        assert_eq!(map.get(hit).unwrap().element_id(), "overlay_0:data_area");
        let hit = map
            .hit_test(&cfg, &FixedMeasure, da.x as f32 + 1.0, mid_y)
            .unwrap();
        assert_eq!(map.get(hit).unwrap().element_id(), "overlay_0:axis_left");
        // A point in the parent's data area still reaches the shared one.
        let parent = cfg.data_area().unwrap().0;
        let hit = map
            .hit_test(
                &cfg,
                &FixedMeasure,
                parent.x as f32 + 50.0,
                parent.y as f32 + 50.0,
            )
            .unwrap();
        assert_eq!(map.get(hit).unwrap().element_id(), "data_area");

        let before = cfg.clone();
        let left = map.find("overlay_0:axis_left").unwrap();
        let drag = map.get(left).unwrap().as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, 9.0, 0.0), NudgeResult::Moved);
        assert_eq!(cfg.overlays[0].y_axis.line_offset, 9.0);
        assert_eq!(cfg.left_y, before.left_y);

        let labels = map.find("overlay_0:tick_labels_bottom").unwrap();
        let bounds = map
            .get(labels)
            .unwrap()
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        assert!(bounds.y > (da.y + da.height) as f32, "{bounds:?}");
        let drag = map.get(labels).unwrap().as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, 2.0, 3.0), NudgeResult::Moved);
        // One X axis: the parent's offset moved, for every area.
        assert_eq!(cfg.bottom_x.label_style.label_offset_x, 2.0);
        assert_eq!(cfg.bottom_x.label_style.label_offset_y, 3.0);
        assert_eq!(cfg.overlays, {
            let mut overlays = before.overlays.clone();
            overlays[0].y_axis.line_offset = 9.0;
            overlays
        });

        // The later layer reaches the same X axis copy through the first
        // layer's panel.
        cfg.set_active_layer(LayerRef::Overlay(1));
        let map = HitMap::for_active_layer(&cfg);
        let axis = map.find("overlay_1:axis_bottom").unwrap();
        let drag = map.get(axis).unwrap().as_draggable().unwrap();
        assert_eq!(drag.drag_by(&mut cfg, 0.0, -4.0), NudgeResult::Moved);
        assert_eq!(cfg.bottom_x.line_offset, -4.0);
        let right = map.find("overlay_1:axis_right").unwrap();
        assert!(
            map.get(right)
                .unwrap()
                .bounds(&cfg, &FixedMeasure)
                .unwrap()
                .x
                >= (da.x + da.width) as f32 - 10.0
        );
    }

    // Dragging a subplot's data area moves its area — both layers of it —
    // carrying sub-pixel deltas; its handles move the area's edges; nothing
    // of the parent moves, and an impossible move is refused atomically.
    #[test]
    fn a_subplot_data_area_moves_and_resizes_its_area() {
        let mut cfg = cfg_with_subplot();
        cfg.set_active_layer(LayerRef::Overlay(1));
        let map = HitMap::for_active_layer(&cfg);
        let id = map.find("overlay_1:data_area").unwrap();
        let element = map.get(id).unwrap();
        let before = cfg.clone();
        let drag = element.as_draggable().unwrap();
        for _ in 0..4 {
            assert_eq!(drag.drag_by(&mut cfg, 0.25, 0.5), NudgeResult::Moved);
        }
        let area = cfg.overlay_area(0).0;
        assert_eq!((area.x, area.y), (1, 602));
        assert_eq!(cfg.overlays[1].chart_area, cfg.overlays[0].chart_area);
        assert_eq!(cfg.chart_area, before.chart_area);
        assert_eq!(cfg.margins(), before.margins());
        assert_eq!(cfg.bottom_x, before.bottom_x);

        let resize = element.as_resizable().unwrap();
        assert!(
            element
                .selection_box(&cfg, &FixedMeasure)
                .unwrap()
                .handles
                .len()
                == 8
        );
        assert_eq!(
            resize.resize_by(&mut cfg, crate::resize::ResizeHandle::SE, 20.0, -50.0),
            NudgeResult::Moved
        );
        let area = cfg.overlay_area(1).0;
        assert_eq!(
            (area.x, area.y, area.width, area.height),
            (1, 602, 820, 350)
        );
        assert_eq!(cfg.overlays[1].chart_area, cfg.overlays[0].chart_area);
        assert_eq!(
            resize.resize_by(&mut cfg, crate::resize::ResizeHandle::W, 5.0, 0.0),
            NudgeResult::Moved
        );
        assert_eq!(cfg.overlay_area(0).0.x, 6);
        assert_eq!(cfg.overlay_area(0).0.width, 815);

        let snapshot = cfg.clone();
        assert_eq!(
            drag.drag_by(&mut cfg, -100.0, 0.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(
            resize.resize_by(&mut cfg, crate::resize::ResizeHandle::N, 0.0, 340.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
        assert_eq!(cfg, snapshot);
        // A layer on the parent has no area of its own to move.
        let mut plain = cfg_with_overlay();
        assert!(
            OverlayDataAreaElement::new(0)
                .bounds(&plain, &FixedMeasure)
                .is_none()
        );
        assert_eq!(
            OverlayDataAreaElement::new(0).drag_by(&mut plain, 5.0, 5.0),
            NudgeResult::Rejected(NudgeReject::OutOfBounds)
        );
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
    fn bottom_axis_bounds_hug_data_area_edge() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisElement { side: Side::Bottom }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        assert_eq!(b.x, da.x as f32);
        assert_eq!(b.width, da.width as f32);
        let axis_y = (da.y + da.height) as f32;
        assert!(b.y <= axis_y && axis_y <= b.y + b.height);
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
    fn bottom_axis_label_strip_hugs_glyph_extent() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisLabelElement { side: Side::Bottom }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Starts past the tick + LABEL_GAP; thickness is the measured glyph
        // height (FixedMeasure: ascent 10 + descent 3), not the whole margin.
        assert_eq!(
            b.y,
            (da.y + da.height) as f32 + cfg.bottom_x.major_tick_length + 4.0
        );
        assert_eq!(b.height, 13.0);
        assert!(b.height < cfg.bottom_x.out_margin);
        assert_eq!(b.x, da.x as f32);
        assert_eq!(b.width, da.width as f32);
    }

    #[test]
    fn left_axis_label_strip_width_scales_with_digits() {
        let cfg = cfg_800x600();
        let da = cfg.data_area().unwrap();
        let b = AxisLabelElement { side: Side::Left }
            .bounds(&cfg, &FixedMeasure)
            .unwrap();
        // Width = (significant_digits + 2) digits × 8 px under FixedMeasure.
        let digits = cfg.left_y.label_style.significant_digits.max(1) as f32 + 2.0;
        assert_eq!(b.width, digits * 8.0);
        // Right edge sits tick + LABEL_GAP inward of the axis line.
        assert_eq!(
            b.x + b.width,
            da.x as f32 - cfg.left_y.major_tick_length - 4.0
        );
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
        let sb2 = AxisElement { side: Side::Bottom }
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
    fn colorbar_axis_labels_and_title_are_independent_hit_targets() {
        for side in [Side::Top, Side::Bottom, Side::Left, Side::Right] {
            let mut cfg = cfg_with_colorbar(side.clone());
            {
                let bar = cfg.colorbar.as_mut().expect("colourbar");
                bar.axis.title_option.visible = true;
                bar.axis.title_option.text =
                    RichText::plain("z", crate::color::Color::BLACK, 12.0, "");
            }
            let map = HitMap::standard_chart();

            let axis = ColorBarAxisElement
                .bounds(&cfg, &FixedMeasure)
                .expect("axis bounds");
            let axis_hit = map
                .hit_test(
                    &cfg,
                    &FixedMeasure,
                    axis.x + axis.width * 0.5,
                    axis.y + axis.height * 0.5,
                )
                .expect("axis hit");
            assert_eq!(map.get(axis_hit).unwrap().element_id(), "colorbar_axis");
            let axis_selection = map
                .selection_box(axis_hit, &cfg, &FixedMeasure)
                .expect("axis selection box");
            assert_eq!(axis_selection.rect, axis.expanded(SELECTION_PADDING));
            assert!(axis_selection.handles.is_empty());

            let labels = ColorBarLabelElement
                .bounds(&cfg, &FixedMeasure)
                .expect("label bounds");
            let label_point = match side {
                Side::Top | Side::Bottom => (labels.x + 1.0, labels.y + labels.height * 0.5),
                Side::Left | Side::Right => (labels.x + labels.width * 0.5, labels.y + 1.0),
            };
            let label_hit = map
                .hit_test(&cfg, &FixedMeasure, label_point.0, label_point.1)
                .expect("label hit");
            assert_eq!(
                map.get(label_hit).unwrap().element_id(),
                "colorbar_tick_labels"
            );
            let label_selection = map
                .selection_box(label_hit, &cfg, &FixedMeasure)
                .expect("label selection box");
            assert_eq!(label_selection.rect, labels.expanded(SELECTION_PADDING));
            assert!(label_selection.handles.is_empty());

            let title = ColorBarTitleElement
                .bounds(&cfg, &FixedMeasure)
                .expect("title bounds");
            let title_hit = map
                .hit_test(
                    &cfg,
                    &FixedMeasure,
                    title.x + title.width * 0.5,
                    title.y + title.height * 0.5,
                )
                .expect("title hit");
            assert_eq!(map.get(title_hit).unwrap().element_id(), "colorbar_title");
            let title_selection = map
                .selection_box(title_hit, &cfg, &FixedMeasure)
                .expect("title selection box");
            assert_eq!(title_selection.rect, title.expanded(SELECTION_PADDING));
            assert!(title_selection.handles.is_empty());
        }
    }

    #[test]
    fn colorbar_detail_bounds_follow_the_painted_strip_offset() {
        let mut cfg = cfg_with_colorbar(Side::Right);
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.length_frac = 0.4;
            bar.align = crate::config::BarAlign::End;
            bar.axis.title_option.visible = true;
            bar.axis.title_option.text =
                RichText::plain("intensity", crate::color::Color::BLACK, 12.0, "");
        }
        let before = [
            ColorBarAxisElement.bounds(&cfg, &FixedMeasure).unwrap(),
            ColorBarLabelElement.bounds(&cfg, &FixedMeasure).unwrap(),
            ColorBarTitleElement.bounds(&cfg, &FixedMeasure).unwrap(),
        ];
        {
            let bar = cfg.colorbar.as_mut().expect("colourbar");
            bar.offset_x += 17.0;
            bar.offset_y -= 11.0;
        }
        let after = [
            ColorBarAxisElement.bounds(&cfg, &FixedMeasure).unwrap(),
            ColorBarLabelElement.bounds(&cfg, &FixedMeasure).unwrap(),
            ColorBarTitleElement.bounds(&cfg, &FixedMeasure).unwrap(),
        ];
        for (before, after) in before.into_iter().zip(after) {
            assert_eq!(after, before.translated(17.0, -11.0));
        }
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
