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
}

impl Default for OverlayLayer {
    /// A right-hand Y axis with the default Y label style, no series and no
    /// legend of its own.
    fn default() -> Self {
        Self {
            y_axis: crate::default::default_axis_options_y(),
            series: Vec::new(),
            legend: None,
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
        let back: OverlayLayer = serde_json::from_value(json).unwrap();
        assert_eq!(back, OverlayLayer::default());

        let layer = OverlayLayer {
            series: vec!["pressure".into()],
            legend: Some(crate::default::default_legend()),
            ..Default::default()
        };
        let json = serde_json::to_value(&layer).unwrap();
        assert_eq!(json["series"], serde_json::json!(["pressure"]));
        assert!(json["legend"].is_object());
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
