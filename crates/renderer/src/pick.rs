//! Pick results, and the CPU twins of the draw shaders' coverage as test
//! support.
//!
//! [`PickedPoint`] and [`PickedData`] are what `Renderer::pick_chart` and
//! `pick_chart_data` resolve to. The CPU picker (`pick_nearest_point` and the
//! twins of the shader functions it mirrors) is test support only: it
//! compiles under the `cpu-twin` feature, which the crate's own tests enable,
//! as `pick::cpu_twin`, from `tests/support/cpu_twin.rs`.

// The style lookups product code shares with the picker live with the kind
// that draws them; `gpu_pick/` still names them here.
// TODO(#38): point `gpu_pick/` at `data_render` and drop these re-exports.
pub(crate) use crate::data_render::vector::vector_anchor_radius_px;
pub(crate) use crate::data_render::{
    MARKER_QUAD_MARGIN_PX, MARKER_STROKE_HALF_PX, marker_base_shape_id, marker_visual_scale,
};

#[cfg(feature = "cpu-twin")]
#[doc(hidden)]
#[path = "../tests/support/cpu_twin.rs"]
pub mod cpu_twin;
#[cfg(all(test, feature = "cpu-twin"))]
pub(crate) use cpu_twin::{PointColumnLookup, PointPickOptions, pick_nearest_point};

#[derive(Debug, Clone, PartialEq)]
pub struct PickedPoint {
    pub source_id: Option<String>,
    pub series_id: String,
    pub point_index: usize,
    pub distance_px: f32,
}

/// One exact data-primitive hit.
///
/// The result contains only source identity and stable indices. Data values,
/// coordinates, bar bounds, and reconstructed contour endpoints remain in the
/// registered columns / chart configuration and are never mirrored onto the
/// CPU by the GPU picker.
#[derive(Debug, Clone, PartialEq)]
pub enum PickedData {
    Point {
        source_id: Option<String>,
        series_id: String,
        point_index: usize,
        distance_px: f32,
    },
    HistogramBin {
        source_id: Option<String>,
        series_id: String,
        bin_index: usize,
        distance_px: f32,
    },
    MatrixCell {
        source_id: Option<String>,
        series_id: String,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
    ContourLevel {
        source_id: Option<String>,
        series_id: String,
        level_index: usize,
        x_index: usize,
        y_index: usize,
        distance_px: f32,
    },
}

impl PickedData {
    pub fn source_id(&self) -> Option<&str> {
        match self {
            Self::Point { source_id, .. }
            | Self::HistogramBin { source_id, .. }
            | Self::MatrixCell { source_id, .. }
            | Self::ContourLevel { source_id, .. } => source_id.as_deref(),
        }
    }

    pub fn series_id(&self) -> &str {
        match self {
            Self::Point { series_id, .. }
            | Self::HistogramBin { series_id, .. }
            | Self::MatrixCell { series_id, .. }
            | Self::ContourLevel { series_id, .. } => series_id,
        }
    }

    pub fn distance_px(&self) -> f32 {
        match self {
            Self::Point { distance_px, .. }
            | Self::HistogramBin { distance_px, .. }
            | Self::MatrixCell { distance_px, .. }
            | Self::ContourLevel { distance_px, .. } => *distance_px,
        }
    }

    pub fn to_ref(&self) -> crate::config::PickedDataRef {
        match self {
            Self::Point {
                source_id,
                series_id,
                point_index,
                ..
            } => crate::config::PickedDataRef::Point {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                point_index: *point_index,
            },
            Self::HistogramBin {
                source_id,
                series_id,
                bin_index,
                ..
            } => crate::config::PickedDataRef::HistogramBin {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                bin_index: *bin_index,
            },
            Self::MatrixCell {
                source_id,
                series_id,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::MatrixCell {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                x_index: *x_index,
                y_index: *y_index,
            },
            Self::ContourLevel {
                source_id,
                series_id,
                level_index,
                x_index,
                y_index,
                ..
            } => crate::config::PickedDataRef::ContourLevel {
                source_id: source_id.clone(),
                series_id: series_id.clone(),
                level_index: *level_index,
                x_index: *x_index,
                y_index: *y_index,
            },
        }
    }

    pub fn into_point(self) -> Option<PickedPoint> {
        let Self::Point {
            source_id,
            series_id,
            point_index,
            distance_px,
        } = self
        else {
            return None;
        };
        Some(PickedPoint {
            source_id,
            series_id,
            point_index,
            distance_px,
        })
    }
}

impl From<PickedPoint> for PickedData {
    fn from(point: PickedPoint) -> Self {
        Self::Point {
            source_id: point.source_id,
            series_id: point.series_id,
            point_index: point.point_index,
            distance_px: point.distance_px,
        }
    }
}
