//! Layer addressing and the annotation layer container.
//!
//! This module is the shared foundation for overlay layers (secondary axes and
//! subplots) and for the dedicated annotation layer. It defines the public
//! SSOT types: the layer address, the annotation container and the overlay
//! layer. Expansion into internal panels lives in [`crate::expand`], hit-map
//! routing in [`crate::select::HitMap::for_active_layer`]; the annotation
//! items arrive with their own feature. Every chart family carries the same
//! `active_layer` / `annotations` pair so one selection API covers all of them.

/// Which layer of a chart document receives pointer interaction.
///
/// This is document state, one level above element selection
/// (`DecorationLayout::selected`, `picked_points`, `picked_data`): it decides
/// which layer's elements are registered in the hit map, not which element is
/// selected. `Base` is the single layer every chart has today, so a document
/// without the field behaves exactly as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum LayerRef {
    /// The chart's own series, axes and decorations.
    #[default]
    Base,
    /// An overlay layer by index into the document's overlay list. Bounded
    /// charts have no overlays; a setter must fall back to `Base` there and
    /// whenever the index is out of range.
    Overlay(usize),
    /// The dedicated annotation layer.
    Annotations,
}

impl LayerRef {
    /// `true` for the default layer; used to omit the field from JSON.
    pub fn is_base(&self) -> bool {
        matches!(self, Self::Base)
    }

    /// The layer as a bounded chart reads it. Bounded charts have no
    /// overlays, so any `Overlay` index falls back to `Base`.
    pub fn bounded(self) -> Self {
        match self {
            Self::Overlay(_) => Self::Base,
            layer => layer,
        }
    }
}

/// The shared `set_active_layer` of the bounded chart families: store the
/// layer with the overlay fallback and, when it actually changes, clear the
/// decoration selection, which names an element of the previous layer's
/// pointer route (a decoration below the annotations, or an
/// `Annotation(index)`). The data selection lives in the chart's own fields
/// and is kept, as `picked_*` is on Cartesian charts.
pub(crate) fn set_bounded_active_layer(
    active: &mut LayerRef,
    decorations: &mut crate::DecorationLayout,
    layer: LayerRef,
) -> LayerRef {
    let layer = layer.bounded();
    if active.bounded() != layer {
        decorations.selected = None;
    }
    *active = layer;
    layer
}

/// The dedicated annotation layer of a chart document.
///
/// Composited as two textures (an underlay below the data and an overlay
/// above every decoration) but toggled, selected and edited as one unit.
/// Items are addressed by index as `DecorationId::Annotation(index)`; see
/// [`crate::annotation`] for the item kinds and the interaction policy.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct AnnotationLayer {
    /// Skips the layer's composite draws without re-rasterizing anything.
    pub visible: bool,
    /// The annotations in draw order. Omitted from JSON when empty.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub items: Vec<crate::annotation::Annotation>,
}

impl Default for AnnotationLayer {
    fn default() -> Self {
        Self {
            visible: true,
            items: Vec::new(),
        }
    }
}

impl AnnotationLayer {
    /// `true` when nothing differs from the default; used to omit the field
    /// from JSON so existing documents serialize byte-identically.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// One overlay layer of a Cartesian document: a second Y axis drawn on the
/// right side of the shared data area, with the series that are scaled by it.
///
/// A layer with its own `chart_area` is drawn in that rectangle instead of
/// on top of the parent: a subplot. Layers whose areas are equal are drawn on
/// top of each other in that area, exactly as layers without an area are
/// drawn on top of the parent. The document covers the union of every area.
///
/// An overlay holds only what may differ from the parent `Config`: its Y
/// axis, the ids of the series it owns and an optional legend of its own. The
/// chart area, both X axes, the chart title, the shared legend, the grid, the
/// colourbar and `draw_style` exist exactly once, on the parent, so the type
/// itself guarantees that a document has one X range and one plot rectangle.
/// Expansion (`Config::expand`) turns the parent and its overlays into one
/// internal panel each; those panels are never serialized.
///
/// Series are referenced by `series_id` from the chart's single series list,
/// so the series declarations, their styles, column references and picking
/// stay where they are today. A series id listed by no overlay belongs to the
/// base layer; an id listed by two overlays is a configuration error.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct OverlayLayer {
    /// The overlay's own Y axis, drawn on the right side of the data area.
    /// Its `min` / `max` / `scale` / `inverted` are the data-to-pixel mapping
    /// of the overlay's series. `out_margin` contributes to the shared right
    /// margin; the group takes the widest band of the parent and every
    /// overlay (`Config::margins`).
    pub y_axis: crate::config::AxisOptions,
    /// Ids of the series this layer scales. Each id may appear in at most one
    /// overlay; ids with no matching series declaration are ignored.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Vec::is_empty"))]
    pub series: Vec<String>,
    /// An optional legend drawn in this layer's panel. `None` (the default)
    /// means the parent's single merged legend lists every layer's series.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub legend: Option<crate::legend::Legend>,
    /// The rectangle this layer is drawn in, in the same surface coordinates
    /// as the parent's `chart_area`. `None` (the default, key absent in JSON)
    /// draws the layer on top of the parent's area, as before the field
    /// existed; an area equal to the parent's is the same thing.
    ///
    /// A layer with an area of its own is a subplot. The first layer of an
    /// area (lowest index) draws its `y_axis` on the left together with the
    /// grid and copies of the parent's two X axes (one X range, one style);
    /// later layers of the same area draw their axes on the right of it, as
    /// overlays of the parent do. The parent's title, merged legend,
    /// colourbar and annotation layer stay in the parent's area.
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub chart_area: Option<crate::layout::ChartArea>,
}

impl Default for OverlayLayer {
    /// A right-hand Y axis with the default Y label style, no series and no
    /// legend of its own.
    fn default() -> Self {
        Self {
            y_axis: crate::default::default_axis_options_y(),
            series: Vec::new(),
            legend: None,
            chart_area: None,
        }
    }
}

impl OverlayLayer {
    /// `true` when this overlay scales `series_id`.
    pub fn owns_series(&self, series_id: &str) -> bool {
        self.series.iter().any(|id| id == series_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_is_the_default_layer() {
        assert_eq!(LayerRef::default(), LayerRef::Base);
        assert!(LayerRef::Base.is_base());
        assert!(!LayerRef::Overlay(0).is_base());
        assert!(!LayerRef::Annotations.is_base());
    }

    #[test]
    fn bounded_layers_fall_back_and_clear_the_decoration_selection_on_change() {
        use crate::decoration::DecorationId;
        assert_eq!(LayerRef::Overlay(0).bounded(), LayerRef::Base);
        assert_eq!(LayerRef::Annotations.bounded(), LayerRef::Annotations);
        let mut active = LayerRef::Base;
        let mut decorations = crate::DecorationLayout {
            selected: Some(DecorationId::Legend),
            ..Default::default()
        };
        // An overlay is stored as `Base`, which is no change: the selection stays.
        assert_eq!(
            set_bounded_active_layer(&mut active, &mut decorations, LayerRef::Overlay(2)),
            LayerRef::Base
        );
        assert_eq!(decorations.selected, Some(DecorationId::Legend));
        assert_eq!(
            set_bounded_active_layer(&mut active, &mut decorations, LayerRef::Annotations),
            LayerRef::Annotations
        );
        assert_eq!(active, LayerRef::Annotations);
        assert_eq!(decorations.selected, None);
        decorations.selected = Some(DecorationId::Annotation(1));
        set_bounded_active_layer(&mut active, &mut decorations, LayerRef::Annotations);
        assert_eq!(decorations.selected, Some(DecorationId::Annotation(1)));
        set_bounded_active_layer(&mut active, &mut decorations, LayerRef::Base);
        assert_eq!(decorations.selected, None);
        // A raw out-of-range value read from JSON counts as `Base`.
        active = LayerRef::Overlay(5);
        decorations.selected = Some(DecorationId::Legend);
        set_bounded_active_layer(&mut active, &mut decorations, LayerRef::Base);
        assert_eq!(active, LayerRef::Base);
        assert_eq!(decorations.selected, Some(DecorationId::Legend));
    }

    // Every bounded family stores its layer through the shared setter, reads
    // an `Overlay` as `Base`, and keeps `Annotations` through JSON.
    #[test]
    fn bounded_charts_set_and_read_their_active_layer() {
        use crate::decoration::DecorationId;
        macro_rules! check {
            ($chart:ty, $data:expr) => {{
                let mut c = <$chart>::default();
                let select_data: fn(&mut $chart) = $data;
                assert_eq!(c.active_layer(), LayerRef::Base);
                c.active_layer = LayerRef::Overlay(1);
                assert_eq!(c.active_layer(), LayerRef::Base);
                c.decorations.selected = Some(DecorationId::Annotation(0));
                select_data(&mut c);
                let with_data = c.clone();
                assert_eq!(c.set_active_layer(LayerRef::Annotations), LayerRef::Annotations);
                assert_eq!(c.decorations.selected, None);
                let mut expected = with_data.clone();
                expected.active_layer = LayerRef::Annotations;
                expected.decorations.selected = None;
                assert_eq!(c, expected, "only the layer and the decoration selection change");
                #[cfg(feature = "serde")]
                {
                    let json = serde_json::to_value(&c).unwrap();
                    assert_eq!(json["active_layer"], serde_json::json!("Annotations"));
                    let back: $chart = serde_json::from_value(json).unwrap();
                    assert_eq!(back, c);
                    let mut legacy = serde_json::to_value(<$chart>::default()).unwrap();
                    assert!(legacy.get("active_layer").is_none());
                    legacy["active_layer"] = serde_json::json!({ "Overlay": 0 });
                    let read: $chart = serde_json::from_value(legacy).unwrap();
                    assert_eq!(read.active_layer(), LayerRef::Base);
                }
            }};
        }
        check!(crate::radial::RadialChart, |c| {
            c.interaction.selected = Some(crate::radial::RadialTarget::main(0))
        });
        check!(crate::categorical::CategoricalChart, |c| {
            c.selected = Some(crate::categorical::CategoryBarTarget::new("a", "s"))
        });
        check!(crate::boxplot::BoxPlotChart, |c| {
            c.selected = Some(crate::boxplot::BoxPlotTarget::new("a", "s"))
        });
    }

    #[test]
    fn annotation_layer_defaults_to_visible() {
        let layer = AnnotationLayer::default();
        assert!(layer.visible);
        assert!(layer.is_default());
        assert!(
            !AnnotationLayer {
                visible: false,
                items: Vec::new()
            }
            .is_default()
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn layer_ref_round_trips_by_name_and_index() {
        for (layer, json) in [
            (LayerRef::Base, r#""Base""#),
            (LayerRef::Overlay(2), r#"{"Overlay":2}"#),
            (LayerRef::Annotations, r#""Annotations""#),
        ] {
            assert_eq!(serde_json::to_string(&layer).unwrap(), json);
            let back: LayerRef = serde_json::from_str(json).unwrap();
            assert_eq!(back, layer);
        }
    }

    #[test]
    fn overlay_layer_defaults_to_an_unassigned_right_axis() {
        let layer = OverlayLayer::default();
        assert_eq!(layer.y_axis, crate::default::default_axis_options_y());
        assert!(layer.series.is_empty());
        assert!(layer.legend.is_none());
        assert!(layer.chart_area.is_none());
        assert!(!layer.owns_series("a"));
        let layer = OverlayLayer {
            series: vec!["a".into()],
            ..Default::default()
        };
        assert!(layer.owns_series("a"));
        assert!(!layer.owns_series("b"));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn overlay_layer_omits_its_optional_keys_and_round_trips() {
        let json = serde_json::to_value(OverlayLayer::default()).unwrap();
        assert!(json.get("y_axis").is_some());
        assert!(json.get("series").is_none(), "{json}");
        assert!(json.get("legend").is_none(), "{json}");
        assert!(json.get("chart_area").is_none(), "{json}");
        let back: OverlayLayer = serde_json::from_value(json).unwrap();
        assert_eq!(back, OverlayLayer::default());

        let layer = OverlayLayer {
            series: vec!["pressure".into()],
            legend: Some(crate::default::default_legend()),
            chart_area: Some(crate::layout::ChartArea(crate::layout::Rect {
                x: 0,
                y: 240,
                width: 640,
                height: 200,
            })),
            ..Default::default()
        };
        let json = serde_json::to_value(&layer).unwrap();
        assert_eq!(json["series"], serde_json::json!(["pressure"]));
        assert!(json["legend"].is_object());
        assert_eq!(
            json["chart_area"],
            serde_json::json!({"x": 0, "y": 240, "width": 640, "height": 200})
        );
        let back: OverlayLayer = serde_json::from_value(json).unwrap();
        assert_eq!(back, layer);
        // A document written with only the axis still parses.
        let partial: OverlayLayer = serde_json::from_str(r#"{"series":["a"]}"#).unwrap();
        assert_eq!(partial.y_axis, OverlayLayer::default().y_axis);
        assert_eq!(partial.series, vec!["a".to_string()]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn annotation_layer_reads_missing_keys_as_defaults() {
        let layer: AnnotationLayer = serde_json::from_str("{}").unwrap();
        assert_eq!(layer, AnnotationLayer::default());
        let hidden: AnnotationLayer = serde_json::from_str(r#"{"visible":false}"#).unwrap();
        assert!(!hidden.visible);
        assert_eq!(
            serde_json::to_string(&hidden).unwrap(),
            r#"{"visible":false}"#
        );
    }
}
