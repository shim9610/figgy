//! Streaming exact GPU picking for [`ColumnPool`](crate::data_render::ColumnPool).
//!
//! Small series scan directly. Larger queries first gate original indices by X
//! and then Y into a compact two-bit mask (scatter point plus line-segment start
//! per index). Only the surviving original indices reach the exact
//! screen-distance calculation. There is no CPU value mirror, index reordering,
//! decimation, or traversal tree. [`GpuPickTicket`] owns its mapping buffer,
//! device, and identity snapshot, so it can outlive both the engine and the
//! chart frame.

use std::fmt;
use std::sync::Arc;

use futures_channel::oneshot;

use crate::data_render::column_pool::PoolIdentity;
use crate::data_render::{
    ColumnHandle, ColumnId, ColumnPool, ScatterStyleMapMeta, ScatterStyleOverrideGpu,
    ScatterStyleSlotGpu, ScatterTransform,
};
use crate::gpu_memory::{
    ChargeTally, GpuByteCharge, GpuLedger, GpuResourceKind, SharedCharge, charged_buffer,
    charged_buffer_init, shared_charge,
};
#[cfg(test)]
use crate::init::observe_result;
use crate::init::{InitEvent, finished, observe_value, started};
use crate::pick::PickedPoint;

mod engine;
mod registry;
mod shader;
pub(crate) mod types;

pub(crate) use engine::GpuPickEngine;
pub use engine::GpuPickTicket;
#[cfg(feature = "cpu-twin")]
pub(crate) use engine::milkyway_line_half_extent;
use engine::query_params;
use engine::{GPU_PICK_DIRECT_SCAN_POINTS, GPU_PICK_WORKGROUP_SIZE, GpuPickTicketState};
use engine::{ReduceResources, create_buffer_checked, gate_dispatch_layout, lane_base};
#[cfg(test)]
use registry::GpuPickSeriesId;
pub(crate) use registry::{GpuPickRegistrySlot, PreparedPickRegistryTransition};
use registry::{GpuPickSeriesIdentities, PickIdentity, PickRegistry, PickSeriesSlot};
use registry::{series_arc, series_exact_variant, stream_exact_variant};
use shader::exact_variant;
#[cfg(test)]
use shader::{INIT_SCOPE, create_pipeline_bundle_observed, validate_device_limits};
pub(crate) use shader::{PickExactKey, PickExactPipelines, PickPipelineBundle, PickShader};
pub use types::GpuPickError;
use types::{CANDIDATE_BYTES, FLAG_LINE, FLAG_SCATTER, FLAG_SIZE_AREA, FLAG_SIZE_MAP};
use types::{FLAG_STYLE_INDEX, FLAG_STYLE_MAP, FLAG_VECTOR, GATE_MASK_BYTES};
pub(crate) use types::{GpuPickDrawStyle, GpuPickQuery, GpuPickScatter, GpuPickScatterStyle};
pub(crate) use types::{GpuPickSeriesDescriptor, GpuPickSizeMap, GpuPickVector, GpuStreamPick};
pub(crate) use types::{GpuStreamPickColumns, validate_stream_handle};
use types::{PICK_LINE_PRECISE, PICK_LINE_RIBBON, PICK_LINE_SKETCH, PICK_MARKER_PLANET};
use types::{PICK_MARKER_PRECISE, PICK_MARKER_SKETCH, PICK_MARKER_STAR, PickCandidateGpu};
use types::{PickQueryParamsGpu, PickSeriesGpu, VECTOR_SCALE_PIXELS, nonnegative};
use types::{storage_entry, style_max_extent, uniform_entry};

#[cfg(test)]
mod tests;
