//! Typed data selections: PickedDataRef, the highlight policy and the wire form.

use super::*;

/// A renderer-independent reference to one selected data primitive.
///
/// These records carry provenance only. Coordinates, bin edges, matrix
/// values, and contour values remain in the caller's registered source
/// columns / series configuration; read them from there.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
pub enum PickedDataRef {
    Point {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
    },
    HistogramBin {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        bin_index: usize,
        /// The half of a split violin (`ViolinSide::Low` or `High`) at the
        /// category `bin_index`; `None` for every other bin, a box, or a
        /// symmetric violin.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        half: Option<crate::data_config::ViolinSide>,
    },
    MatrixCell {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// Canonical x-axis cell/quad index, independent of matrix storage
        /// orientation.
        x_index: usize,
        /// Canonical y-axis cell/quad index, independent of matrix storage
        /// orientation.
        y_index: usize,
    },
    ContourLevel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// Index into `ContourConfig.levels`; the level value itself stays in
        /// the chart configuration SSoT.
        level_index: usize,
        /// Canonical x/y sample-cell indices containing the hit. They identify
        /// the local piece without copying a reconstructed coordinate.
        x_index: usize,
        y_index: usize,
    },
    /// The value label of one category of a category bar, box plot or
    /// radar series: its `labels.text[category_index]`, drawn at the bar
    /// end, above the box or outside the vertex. A selected label is
    /// outlined by its frame and is what a drag moves
    /// (`SeriesConfig::drag_value_label`); `step_selection` does not step
    /// it.
    ValueLabel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        /// The category, in axis order: the index into `labels.text`. On an
        /// energy-level series, the row in the list `part` names.
        category_index: usize,
        /// On an energy-level series, the list the label belongs to (#203);
        /// `None` for every other series.
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        part: Option<crate::energy_levels::LabelPart>,
    },
    /// One triangle of a triangle mesh series (#210): its row in the
    /// triangle index column, in triangle order. The vertex nearest the
    /// cursor is a hover answer and is not stored.
    MeshTriangle {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        triangle_index: usize,
    },
    /// A vertex of a Feynman series (#234): its row in `vertices`. A drag
    /// moves it (`FeynmanSeries::drag_vertex`).
    FeynmanVertex {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        vertex_index: usize,
    },
    /// A propagator of a Feynman series (#234): its row in `propagators`.
    /// A drag bends it (`FeynmanSeries::drag_bend`).
    FeynmanPropagator {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        propagator_index: usize,
    },
    /// A label of a Feynman series (#234): the row `index` of the list
    /// `part` names. A drag moves it (`FeynmanSeries::drag_label`);
    /// `step_selection` does not step it.
    FeynmanLabel {
        #[cfg_attr(
            feature = "serde",
            serde(default, skip_serializing_if = "Option::is_none")
        )]
        source_id: Option<String>,
        series_id: String,
        part: crate::feynman::FeynmanLabelPart,
        index: usize,
    },
}

impl PickedDataRef {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. }
            | Self::ValueLabel { source_id, .. }
            | Self::MeshTriangle { source_id, .. }
            | Self::FeynmanVertex { source_id, .. }
            | Self::FeynmanPropagator { source_id, .. }
            | Self::FeynmanLabel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. }
            | Self::ValueLabel { series_id, .. }
            | Self::MeshTriangle { series_id, .. }
            | Self::FeynmanVertex { series_id, .. }
            | Self::FeynmanPropagator { series_id, .. }
            | Self::FeynmanLabel { series_id, .. } => series_id,
        }
    }
}

/// Visual policy for typed data selections.
///
/// The selected records are small stable identities. Every overlay resolves
/// its geometry from the same current `Config` and `ColumnPool` used by the
/// normal draw, so an axis/range update cannot leave a stored CPU rectangle or
/// contour segment behind.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(default = "crate::defaults::data_selections_config")
)]
pub struct DataSelectionsConfig {
    pub visible: bool,
    pub refs: Vec<PickedDataRef>,
    pub highlight_color: Color,
    pub outline_width_px: f32,
    pub point_radius_extra_px: f32,
    pub contour_width_extra_px: f32,
}

impl DataSelectionsConfig {
    /// No selection, with the default highlight look (#266).
    pub fn empty() -> Self {
        crate::defaults::data_selections_config()
    }
}

/// The document form of [`Config::picked_data`]: the `picked_data` key, and
/// on reading also the `picked_points` key documents wrote before 0.19.
#[cfg(feature = "serde")]
pub(super) mod picked_data_wire {
    use super::{Color, DataSelectionsConfig, PickedDataRef};

    /// The point selection documents wrote before 0.19, read only to be
    /// folded into `picked_data`.
    #[derive(serde::Deserialize)]
    #[serde(default)]
    struct LegacyPickedPoints {
        visible: bool,
        refs: Vec<LegacyPointRef>,
        ring_color: Color,
        ring_width_px: f32,
        radius_extra_px: f32,
    }

    impl Default for LegacyPickedPoints {
        fn default() -> Self {
            let highlight = DataSelectionsConfig::empty();
            Self {
                visible: highlight.visible,
                refs: Vec::new(),
                ring_color: highlight.highlight_color,
                ring_width_px: highlight.outline_width_px,
                radius_extra_px: highlight.point_radius_extra_px,
            }
        }
    }

    #[derive(serde::Deserialize)]
    struct LegacyPointRef {
        #[serde(default)]
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
    }

    #[derive(serde::Deserialize)]
    struct Wire {
        #[serde(default)]
        picked_points: Option<LegacyPickedPoints>,
        #[serde(default)]
        picked_data: Option<DataSelectionsConfig>,
    }

    #[derive(serde::Serialize)]
    struct WireRef<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        picked_data: Option<&'a DataSelectionsConfig>,
    }

    pub(in crate::config) fn serialize<S: serde::Serializer>(
        picked_data: &Option<DataSelectionsConfig>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(
            &WireRef {
                picked_data: picked_data.as_ref(),
            },
            serializer,
        )
    }

    pub(in crate::config) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<DataSelectionsConfig>, D::Error> {
        let Wire {
            picked_points,
            picked_data,
        } = <Wire as serde::Deserialize>::deserialize(deserializer)?;
        let Some(legacy) = picked_points else {
            return Ok(picked_data);
        };
        let points = legacy.refs.into_iter().map(|point| PickedDataRef::Point {
            source_id: point.source_id,
            series_id: point.series_id,
            point_index: point.point_index,
        });
        Ok(Some(match picked_data {
            // The typed selection keeps its own look; the points join it
            // when they were shown.
            Some(mut selection) => {
                if legacy.visible {
                    selection.refs.extend(points);
                }
                selection
            }
            None => DataSelectionsConfig {
                visible: legacy.visible,
                refs: points.collect(),
                highlight_color: legacy.ring_color,
                outline_width_px: legacy.ring_width_px,
                point_radius_extra_px: legacy.radius_extra_px,
                // The legacy points had no contour to widen.
                contour_width_extra_px: DataSelectionsConfig::empty().contour_width_extra_px,
            },
        }))
    }
}
