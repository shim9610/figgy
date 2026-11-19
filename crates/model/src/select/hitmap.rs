//! The hit map: registering selectable elements and hit-testing a point.

use super::*;

/// Id of an entry registered in a [`HitMap`]. Stable for the map's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitId(pub(super) usize);

/// Registry of selectable elements for one chart panel + point hit-testing.
///
/// Hosts feed pointer events here: `hit_test` returns the element under the
/// point (later registrations win ties — register background elements like
/// the data area first, small foreground elements like titles last). Where
/// the selection padding of several elements reaches the point, the one the
/// pointer is visually on wins ([`Self::hit_test`]). The model owns this
/// because hit-testing is pure bounds geometry; the renderer only supplies
/// the [`MeasureText`] implementation.
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

    /// The element under `(x, y)`: the one the pointer is visually on
    /// among the elements whose hit region ([`SelectionGeometry::contains`],
    /// the selection padding included) holds the point (#272). Hit testing,
    /// selection and the start of a drag all come through here, so they
    /// share the decision.
    ///
    /// - An element whose own, unpadded geometry holds the point wins over
    ///   one that reaches it only with its padding; among those that reach
    ///   it only with their padding, the one whose geometry is nearest wins
    ///   ([`SelectionGeometry::hit_distance`]). A point within
    ///   [`INK_FRINGE`] (1 px) of an element's geometry counts as on it:
    ///   its antialiased edge reaches that far.
    /// - Ties keep the topmost: entries are tested in reverse registration
    ///   order and a later one is taken only when it is strictly nearer.
    /// - Backgrounds ([`Selectable::is_background`]: the data area, a
    ///   subplot's frame) take no part: the first background that holds
    ///   the point ends the search, answering only when no element above
    ///   it does. So a label's padding still grabs the label over the plot,
    ///   and what a background covers stays hidden under it.
    ///
    /// One pass that keeps only the best candidate: no list of candidates
    /// is built.
    pub fn hit_test(&self, cfg: &C, measure: &dyn MeasureText, x: f32, y: f32) -> Option<HitId> {
        // The nearest element so far, topmost first: its index and how far
        // the point stands from its own geometry.
        let mut best: Option<(usize, f32)> = None;
        for (i, el) in self.entries.iter().enumerate().rev() {
            if !el.owns_point(cfg, x, y) {
                continue;
            }
            if el.is_background() {
                if el.contains(cfg, measure, x, y) {
                    return Some(HitId(best.map_or(i, |(b, _)| b)));
                }
                continue;
            }
            let Some(distance) = el
                .hit_distance(cfg, measure, x, y)
                .map(|d| (d - INK_FRINGE).max(0.0))
            else {
                continue;
            };
            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some((i, distance));
            }
            // On the element's own geometry: nothing below can be nearer.
            if distance <= 0.0 {
                break;
            }
        }
        best.map(|(i, _)| HitId(i))
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
        // The x axes' category group headers (#203): registered on every
        // chart, so the set stays the active layer's; an axis without
        // groups places nothing and is never hit.
        for side in [Side::Top, Side::Bottom] {
            map.register(CategoryGroupLabelElement { side });
        }
        map
    }

    /// The registration set for the layer `cfg.active_layer()` addresses.
    ///
    /// `Base` is exactly [`Self::standard_chart`]. For any other layer the
    /// shared elements come first — data area, the parent's axes the layer
    /// shares with their label bands and titles, the merged legend, the
    /// chart title and the colourbar chrome — and the active layer's own
    /// elements last, so the reverse-order hit test puts them on top. An
    /// `Overlay(i)` shares the parent's two X axes, or only the bottom one
    /// while it has an X axis of its own (`link_x` off), and the parent's
    /// left Y axis while it follows it (`link_y` on). Its own elements are
    /// its own X axis on the top side, its own Y axis on the right, each with
    /// its label band and title, and its per-layer legend
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
    /// The set depends only on the active layer, the overlay count, the
    /// active subplot's overlay count and the active overlay's link flags, so
    /// a host rebuilds the map when any of them changes; `HitId`s from a previous map are not valid in the new
    /// one.
    pub fn for_active_layer(cfg: &Config) -> Self {
        let layer = cfg.active_layer();
        if layer == LayerRef::Base {
            let mut map = Self::standard_chart();
            // A parallel chart's axes (#141), each its own label band, over
            // the shared elements.
            if let Some(parallel) = cfg.parallel() {
                for axis in 0..parallel.axes.len() {
                    map.register(ParallelAxisLabelElement { axis });
                }
            }
            // A polar chart's category labels (#309), each its own element
            // over the angle labels as a whole.
            if cfg.polar().is_some()
                && let Some(categories) = cfg.bottom_x.categories.as_deref()
            {
                for index in 0..categories.len() {
                    map.register(CategoryLabelElement { index });
                }
            }
            map.register_subplots(cfg);
            return map;
        }
        if let LayerRef::Subplot(index) | LayerRef::SubplotOverlay(index, _) = layer {
            return Self::for_subplot(cfg, index, layer);
        }
        // The parent's axes this layer shares: both X axes unless an
        // overlay has an X axis of its own (its top side is then the
        // overlay's), and the left Y axis when an overlay follows it.
        let overlay = match layer {
            LayerRef::Overlay(index) => cfg.overlays.get(index),
            _ => None,
        };
        let own_x = overlay.is_some_and(|overlay| !overlay.link_x);
        let linked_y = overlay.is_some_and(|overlay| overlay.link_y);
        let shared: &[Side] = match (own_x, linked_y) {
            (false, false) => &[Side::Top, Side::Bottom],
            (false, true) => &[Side::Top, Side::Bottom, Side::Left],
            (true, false) => &[Side::Bottom],
            (true, true) => &[Side::Bottom, Side::Left],
        };
        let mut map = Self::new();
        map.register(DataAreaElement);
        for side in shared {
            map.register(AxisElement { side: side.clone() });
        }
        for side in shared {
            map.register(AxisLabelElement { side: side.clone() });
        }
        for side in shared {
            map.register(AxisTitleElement { side: side.clone() });
        }
        map.register(LegendElement);
        map.register(ChartTitleElement);
        map.register(ColorBarElement);
        map.register(ColorBarAxisElement);
        map.register(ColorBarLabelElement);
        map.register(ColorBarTitleElement);
        // The shared x axes' group headers are shared too (#203).
        for side in shared
            .iter()
            .filter(|side| matches!(side, Side::Top | Side::Bottom))
        {
            map.register(CategoryGroupLabelElement { side: side.clone() });
        }
        // A polar chart's category labels (#309) go with the shared angle
        // labels (the annotation layer; polar charts have no overlays).
        if shared.contains(&Side::Bottom)
            && cfg.polar().is_some()
            && let Some(categories) = cfg.bottom_x.categories.as_deref()
        {
            for index in 0..categories.len() {
                map.register(CategoryLabelElement { index });
            }
        }
        if let LayerRef::Overlay(index) = layer {
            if own_x {
                map.register(OverlayXAxisElement { layer: index });
                map.register(OverlayXAxisLabelElement { layer: index });
                map.register(OverlayXAxisTitleElement { layer: index });
            }
            if !linked_y {
                map.register(OverlayAxisElement { layer: index });
                map.register(OverlayAxisLabelElement { layer: index });
                map.register(OverlayAxisTitleElement { layer: index });
            }
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
    /// Every subplot on the base layer (#205), over the parent's elements:
    /// its frame ([`SubplotFrameElement`], which a press on its plot
    /// selects) and its own chart's base set except its plot, wrapped as
    /// [`SubplotElement`]s that answer only where their subplot is drawn on
    /// top ([`Config::subplot_at`]). The registration does not depend on
    /// where the subplots are, so a frame moved over or out of another
    /// panel keeps its ids.
    fn register_subplots(&mut self, cfg: &Config) {
        for index in 0..cfg.subplots.len() {
            self.register(SubplotFrameElement { subplot: index });
            let Some(mut subplot) = cfg.subplot_config(index) else {
                continue;
            };
            subplot.active_layer = LayerRef::Base;
            for element in Self::for_active_layer(&subplot).entries {
                if element.element_id() == "data_area" {
                    continue;
                }
                self.register(SubplotElement {
                    subplot: index,
                    inner: element,
                    routed: true,
                });
            }
        }
    }

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
                routed: false,
            });
        }
        map
    }
}
