//! Layer addressing and the annotation layer container.
//!
//! This module is the shared foundation for overlay layers (secondary axes and
//! subplots) and for the dedicated annotation layer. It only defines the
//! public SSOT types; expansion into internal panels, hit-map routing and the
//! annotation items themselves arrive with their own features. Every chart
//! family carries the same two fields so one selection API covers all of them.

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
/// The annotation items are added by the annotation feature; this container
/// exists so every chart family already carries the layer and its visibility.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(default)
)]
pub struct AnnotationLayer {
    /// Skips the layer's composite draws without re-rasterizing anything.
    pub visible: bool,
}

impl Default for AnnotationLayer {
    fn default() -> Self {
        Self { visible: true }
    }
}

impl AnnotationLayer {
    /// `true` when nothing differs from the default; used to omit the field
    /// from JSON so existing documents serialize byte-identically.
    pub fn is_default(&self) -> bool {
        *self == Self::default()
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
        assert!(!AnnotationLayer { visible: false }.is_default());
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
