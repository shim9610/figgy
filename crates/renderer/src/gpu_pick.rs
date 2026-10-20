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

const INIT_SCOPE: &str = "renderer.gpu_pick";

/// Logical point/segment indices represented by one gate-mask word.
const GPU_PICK_GATE_WORD_POINTS: u32 = 32;
/// Compute workgroup width used by gating, exact testing, and reduction.
const GPU_PICK_WORKGROUP_SIZE: u32 = 64;
/// Up to 32 workgroups scan directly; larger series use the X/Y gates first.
const GPU_PICK_DIRECT_SCAN_POINTS: u32 = GPU_PICK_GATE_WORD_POINTS * GPU_PICK_WORKGROUP_SIZE * 32;

const GATE_MASK_BYTES: u64 = 8;
const CANDIDATE_BYTES: u64 = 32;
/// `QUAD_MARGIN_PX` of the shared marker shape (SHADER_COMMON.md §5.5): a
/// marker draws nothing farther than its visual radius plus this margin from
/// its centre on either axis.
const MARKER_QUAD_MARGIN_PX: f32 = crate::pick::MARKER_QUAD_MARGIN_PX;

const FLAG_SCATTER: u32 = 1;
const FLAG_LINE: u32 = 2;
const FLAG_STYLE_MAP: u32 = 4;
const FLAG_STYLE_INDEX: u32 = 8;
/// A `point_size_map` is bound: the exact pass resolves each point's radius
/// from its size value, and the gate extent includes the map's `max_px`.
const FLAG_SIZE_MAP: u32 = 16;
/// The size map scales marker area rather than radius.
const FLAG_SIZE_AREA: u32 = 32;
/// The scatter primitive is a point vector's arrows (`PICK_FLAG_VECTOR`).
const FLAG_VECTOR: u32 = 64;
/// The exact pass's `PICK_VARIANT` bits (`gpu_pick/core.wgsl`): which
/// optional paths one `pick_exact_candidates` pipeline carries.
pub(crate) mod exact_variant {
    pub(crate) const STYLED_MARKER: u32 = 2;
    pub(crate) const DASH: u32 = 16;
    pub(crate) const STYLED_LINE: u32 = 32;
    pub(crate) const POLAR: u32 = 64;
    /// Every path: the shader's default, and what a streamed pick uses.
    pub(crate) const ALL: u32 = STYLED_MARKER | DASH | STYLED_LINE | POLAR;
    /// The override's name in `gpu_pick/core.wgsl`.
    pub(crate) const CONSTANT: &str = "PICK_VARIANT";
}

/// The assembled point-picker shaders (rule 9's pattern, as in
/// `data_render::vector`): the gate module and one exact module per
/// primitive class, each the concatenation of its fragments under
/// `src/gpu_pick/` in the order [`PickShader::fragments`] lists. A pick
/// compiles only the exact module (and variant) its series need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickShader {
    /// The gate and reduction entries.
    Gate,
    /// Exact: a marker series.
    Marker,
    /// Exact: a series with markers and a line.
    MarkerLine,
    /// Exact: a line series.
    Line,
    /// Exact: a point vector series.
    Vector,
}

macro_rules! pick_fragment {
    ($name:literal) => {
        include_str!(concat!("gpu_pick/", $name))
    };
}

macro_rules! pick_module {
    ($($name:literal),+ $(,)?) => {
        concat!($(pick_fragment!($name), "\n"),+)
    };
}

impl PickShader {
    #[cfg(test)]
    pub(crate) const EXACT: [PickShader; 4] = [
        PickShader::Marker,
        PickShader::MarkerLine,
        PickShader::Line,
        PickShader::Vector,
    ];

    /// The fragment files, in assembly order.
    #[cfg(test)]
    pub(crate) fn fragments(self) -> &'static [&'static str] {
        match self {
            PickShader::Gate => &["core.wgsl", "gate_entries.wgsl"],
            PickShader::Marker => &[
                "core.wgsl",
                "noise.wgsl",
                "marker.wgsl",
                "no_line.wgsl",
                "exact_entry.wgsl",
            ],
            PickShader::MarkerLine => &[
                "core.wgsl",
                "noise.wgsl",
                "marker.wgsl",
                "line.wgsl",
                "exact_entry.wgsl",
            ],
            PickShader::Line => &[
                "core.wgsl",
                "noise.wgsl",
                "no_scatter.wgsl",
                "line.wgsl",
                "exact_entry.wgsl",
            ],
            PickShader::Vector => &[
                "core.wgsl",
                "vector.wgsl",
                "no_line.wgsl",
                "exact_entry.wgsl",
            ],
        }
    }

    /// The assembled source: [`Self::fragments`] concatenated, each followed
    /// by one newline.
    pub(crate) const fn source(self) -> &'static str {
        match self {
            PickShader::Gate => pick_module!("core.wgsl", "gate_entries.wgsl"),
            PickShader::Marker => pick_module!(
                "core.wgsl",
                "noise.wgsl",
                "marker.wgsl",
                "no_line.wgsl",
                "exact_entry.wgsl",
            ),
            PickShader::MarkerLine => pick_module!(
                "core.wgsl",
                "noise.wgsl",
                "marker.wgsl",
                "line.wgsl",
                "exact_entry.wgsl",
            ),
            PickShader::Line => pick_module!(
                "core.wgsl",
                "noise.wgsl",
                "no_scatter.wgsl",
                "line.wgsl",
                "exact_entry.wgsl",
            ),
            PickShader::Vector => pick_module!(
                "core.wgsl",
                "vector.wgsl",
                "no_line.wgsl",
                "exact_entry.wgsl",
            ),
        }
    }

    fn label(self) -> &'static str {
        match self {
            PickShader::Gate => "figgy GPU pick gate shader",
            PickShader::Marker => "figgy GPU pick exact marker shader",
            PickShader::MarkerLine => "figgy GPU pick exact marker-line shader",
            PickShader::Line => "figgy GPU pick exact line shader",
            PickShader::Vector => "figgy GPU pick exact vector shader",
        }
    }

    fn exact_index(self) -> usize {
        match self {
            PickShader::Gate => unreachable!("the gate module has no exact entry"),
            PickShader::Marker => 0,
            PickShader::MarkerLine => 1,
            PickShader::Line => 2,
            PickShader::Vector => 3,
        }
    }
}

/// One exact pipeline: the module, the `PICK_VARIANT` bits and the
/// `PICK_SHAPE` the driver specialises it for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PickExactKey {
    pub(crate) shader: PickShader,
    pub(crate) bits: u32,
    /// A marker shape id, or [`PickExactKey::ANY_SHAPE`].
    pub(crate) shape: u32,
}

impl PickExactKey {
    /// `PICK_SHAPE_ANY`: the shape is read per point.
    pub(crate) const ANY_SHAPE: u32 = u32::MAX;

    /// What a streamed pick scans every chunk with: the marker-and-line
    /// module with every path and any shape (streaming is not specialised).
    pub(crate) const STREAMED: Self = Self {
        shader: PickShader::MarkerLine,
        bits: exact_variant::ALL,
        shape: Self::ANY_SHAPE,
    };
}

/// The exact pipelines created so far. Each is created on first use (the
/// first pick of a series that needs it) and kept for the renderer's
/// lifetime; owned by the renderer's picker and passed to each pick, so no
/// lock guards it. `pipelines` grows by one entry per distinct key a chart
/// has needed (a handful: module x draw-style paths x marker shape).
#[derive(Default)]
pub(crate) struct PickExactPipelines {
    modules: [Option<wgpu::ShaderModule>; 4],
    pipelines: Vec<(PickExactKey, wgpu::ComputePipeline)>,
}

impl PickExactPipelines {
    /// The exact pipeline for `key`, created if it is not yet.
    pub(crate) fn ensure(
        &mut self,
        bundle: &PickPipelineBundle,
        key: PickExactKey,
    ) -> Result<&wgpu::ComputePipeline, GpuPickError> {
        let index = match self.pipelines.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                self.pipelines
                    .try_reserve(1)
                    .map_err(|_| GpuPickError::AllocationFailed {
                        resource: "exact pick pipelines",
                    })?;
                let device = &bundle.device;
                let shader = key.shader;
                let module = self.modules[shader.exact_index()].get_or_insert_with(|| {
                    crate::gpu_compile::shader_module(
                        device,
                        wgpu::ShaderModuleDescriptor {
                            label: Some(shader.label()),
                            source: wgpu::ShaderSource::Wgsl(shader.source().into()),
                        },
                    )
                });
                let base = crate::gpu_compile::workgroup_options(device);
                let mut constants = [(exact_variant::CONSTANT, f64::from(key.bits)); 3];
                let mut count = 1;
                // Only the marker modules declare the shape override.
                if key.shape != PickExactKey::ANY_SHAPE
                    && matches!(shader, PickShader::Marker | PickShader::MarkerLine)
                {
                    constants[count] = ("PICK_SHAPE", f64::from(key.shape));
                    count += 1;
                }
                if let Some(&zero) = base.constants.first() {
                    constants[count] = zero;
                    count += 1;
                }
                let pipeline = crate::gpu_compile::compute_pipeline(
                    device,
                    &wgpu::ComputePipelineDescriptor {
                        label: Some("figgy GPU pick exact candidate pipeline"),
                        layout: Some(&bundle.exact_layout),
                        module,
                        entry_point: Some("pick_exact_candidates"),
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants: &constants[..count],
                            zero_initialize_workgroup_memory: base.zero_initialize_workgroup_memory,
                        },
                        cache: None,
                    },
                );
                self.pipelines.push((key, pipeline));
                self.pipelines.len() - 1
            }
        };
        Ok(&self.pipelines[index].1)
    }

    fn get(&self, key: PickExactKey) -> Option<&wgpu::ComputePipeline> {
        self.pipelines
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, pipeline)| pipeline)
    }
}

/// The exact module and variant that pick `series` as the query draws it:
/// its primitive class, the styled marker and line paths when the draw
/// style uses them, the dash walk for a dashed line and the polar paths on
/// polar axes.
fn series_exact_variant(series: &PickSeriesGpu, query: &GpuPickQuery) -> PickExactKey {
    let scatter = series.flags & FLAG_SCATTER != 0;
    let line = series.flags & FLAG_LINE != 0;
    let shader = if series.flags & FLAG_VECTOR != 0 {
        PickShader::Vector
    } else {
        match (scatter, line) {
            (true, true) => PickShader::MarkerLine,
            (false, true) => PickShader::Line,
            _ => PickShader::Marker,
        }
    };
    let mut bits = 0;
    if scatter && shader != PickShader::Vector && query.draw_style.marker != PICK_MARKER_PRECISE {
        bits |= exact_variant::STYLED_MARKER;
    }
    if line {
        if series.line_dash_len > 0 {
            bits |= exact_variant::DASH;
        }
        if query.draw_style.line != PICK_LINE_PRECISE {
            bits |= exact_variant::STYLED_LINE;
        }
    }
    if query.transform.polar_angle[0] > 0.5 {
        bits |= exact_variant::POLAR;
    }
    // Without a style map every point has the series' shape.
    let shape = if scatter && series.flags & FLAG_STYLE_MAP == 0 {
        series.base_shape_id
    } else {
        PickExactKey::ANY_SHAPE
    };
    PickExactKey {
        shader,
        bits,
        shape,
    }
}

/// `PICK_VECTOR_SCALE_PIXELS` in `PickQueryParams.vector.w`.
const VECTOR_SCALE_PIXELS: u32 = 256;
const STYLE_MASK_RADIUS: u32 = 2;
const STYLE_MASK_SHAPE: u32 = 4;
/// The pick shader's marker draw modes (`PICK_MARKER_*`).
const PICK_MARKER_PRECISE: u32 = 0;
const PICK_MARKER_SKETCH: u32 = 1;
const PICK_MARKER_PLANET: u32 = 2;
const PICK_MARKER_STAR: u32 = 3;
/// The pick shader's line draw modes (`PICK_LINE_*`).
const PICK_LINE_PRECISE: u32 = 0;
const PICK_LINE_SKETCH: u32 = 1;
const PICK_LINE_RIBBON: u32 = 2;

#[derive(Clone, Debug)]
pub enum GpuPickError {
    PickerDisabled,
    UnknownChart(crate::renderer::ChartId),
    DeviceLimit {
        resource: &'static str,
        requested: u64,
        limit: u64,
    },
    AllocationFailed {
        resource: &'static str,
    },
    AsyncCompileFailed(String),
    MissingColumn(ColumnId),
    ColumnNotResident(ColumnId),
    StaleColumn {
        series_id: String,
        column_id: ColumnId,
    },
    EmptySeries(String),
    NoPickPrimitive(String),
    InvalidStyleMap {
        series_id: String,
        reason: &'static str,
    },
    TooManyValues {
        series_id: String,
        count: usize,
    },
    /// Retained for source compatibility with the former traversal picker.
    /// The streaming gate implementation never emits this error.
    TraversalOverflow,
    MapChannelClosed,
    MapFailed(wgpu::BufferAsyncError),
    InvalidGpuResult,
    InvalidSeriesIndex {
        index: usize,
        len: usize,
    },
    DuplicateSeriesIndex {
        index: usize,
    },
    ForeignColumnPool,
    RegistryGenerationExhausted,
}

impl fmt::Display for GpuPickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PickerDisabled => write!(f, "GPU picking is not enabled for this renderer"),
            Self::UnknownChart(id) => write!(f, "unknown GPU pick chart id: {id:?}"),
            Self::DeviceLimit {
                resource,
                requested,
                limit,
            } => write!(
                f,
                "GPU pick {resource} exceeds a device limit: requested {requested}, limit {limit}"
            ),
            Self::AllocationFailed { resource } => {
                write!(f, "GPU pick could not allocate {resource}")
            }
            Self::AsyncCompileFailed(reason) => {
                write!(f, "GPU pick pipeline compile failed: {reason}")
            }
            Self::MissingColumn(id) => write!(f, "GPU pick column is missing: {id}"),
            Self::ColumnNotResident(id) => {
                write!(f, "GPU pick column is registered but not resident: {id}")
            }
            Self::StaleColumn {
                series_id,
                column_id,
            } => write!(
                f,
                "GPU picker for series {series_id:?} is stale because column {column_id:?} moved, was removed, or changed length"
            ),
            Self::EmptySeries(id) => write!(f, "GPU pick series {id:?} has no paired values"),
            Self::NoPickPrimitive(id) => {
                write!(
                    f,
                    "GPU pick series {id:?} has neither scatter nor line geometry"
                )
            }
            Self::InvalidStyleMap { series_id, reason } => {
                write!(
                    f,
                    "GPU pick style map for series {series_id:?} is invalid: {reason}"
                )
            }
            Self::TooManyValues { series_id, count } => write!(
                f,
                "GPU pick series {series_id:?} has {count} values; the WebGPU storage index limit is {}",
                u32::MAX
            ),
            Self::TraversalOverflow => write!(f, "legacy GPU pick traversal overflow"),
            Self::MapChannelClosed => write!(f, "GPU pick readback callback was dropped"),
            Self::MapFailed(error) => write!(f, "GPU pick readback mapping failed: {error:?}"),
            Self::InvalidGpuResult => write!(f, "GPU pick returned an invalid scalar record"),
            Self::InvalidSeriesIndex { index, len } => write!(
                f,
                "GPU pick series index {index} is out of bounds for {len} registered series"
            ),
            Self::DuplicateSeriesIndex { index } => {
                write!(
                    f,
                    "GPU pick series index {index} occurs more than once in a batch"
                )
            }
            Self::ForeignColumnPool => {
                write!(f, "GPU picker cannot use a different ColumnPool instance")
            }
            Self::RegistryGenerationExhausted => {
                write!(f, "GPU pick registry generation is exhausted")
            }
        }
    }
}

impl std::error::Error for GpuPickError {}

/// Exact per-point scatter-style inputs.
///
/// These are the small arrays already derived from `SeriesConfig` for the
/// precise render path.  They are uploaded once and cached with the series.
/// Passing `None` for [`GpuPickSeriesDescriptor::scatter`] disables scatter
/// picking.  Passing `style_map: None` selects the production picker's
/// non-mapped/base-style semantics (used by non-precise draw styles).
pub(crate) struct GpuPickScatterStyle<'a> {
    pub(crate) style_index_column: Option<ColumnId>,
    pub(crate) style_slots: &'a [ScatterStyleSlotGpu],
    pub(crate) style_overrides: &'a [ScatterStyleOverrideGpu],
    pub(crate) style_meta: ScatterStyleMapMeta,
}

/// A series' `point_size_map`, as the pick shader needs it: the column the
/// size values come from and the explicit domain → pixel mapping. Twin of
/// `PointSizeMap`; the radius is re-derived per point in the exact pass.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GpuPickSizeMap {
    pub(crate) column: ColumnId,
    pub(crate) domain_min: f64,
    pub(crate) domain_max: f64,
    pub(crate) min_px: f32,
    pub(crate) max_px: f32,
    pub(crate) area: bool,
}

/// Scatter inputs for one registered series.
pub(crate) struct GpuPickScatter<'a> {
    pub(crate) base_radius_px: f32,
    pub(crate) base_shape_id: u32,
    /// `Some` only when current precise-mode style mapping is active.
    pub(crate) style_map: Option<GpuPickScatterStyle<'a>>,
    /// `Some` only in precise mode with a `point_size_map`.
    pub(crate) size_map: Option<GpuPickSizeMap>,
}

/// Registration-time metadata for one exact GPU-picked series.
pub(crate) struct GpuPickSeriesDescriptor<'a> {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) x_column: ColumnId,
    pub(crate) y_column: ColumnId,
    pub(crate) scatter: Option<GpuPickScatter<'a>>,
    /// Full production picker line width.  The engine applies `max(0) / 2`.
    pub(crate) line_width_px: Option<f32>,
    /// How the line connects its samples. A step mode makes the picker walk
    /// the same axis-aligned pieces the line shader draws.
    pub(crate) line_interpolation: crate::data_config::LineInterpolation,
    /// The line's dash preset: the picker walks the same dash phase the line
    /// shader draws, so a dash gap does not pick.
    pub(crate) line_style: crate::line::LineStylePreset,
    /// A point vector series: its markers are the arrows drawn from each
    /// anchor (`scatter` must be set too, for the scatter primitive slot).
    pub(crate) vector: Option<GpuPickVector>,
}

/// The arrows of a point vector series, as the vector shader draws them
/// (`vector.rs::VectorParamsGpu`): the component columns, the columns that
/// bound how many arrows are drawn, and the style in logical pixels.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GpuPickVector {
    pub(crate) u_column: ColumnId,
    pub(crate) v_column: ColumnId,
    /// A colour column: the vector shader draws only as many arrows as it
    /// has values.
    pub(crate) color_column: Option<ColumnId>,
    /// `VectorScale::Pixels` (else `Data`).
    pub(crate) scale_pixels: bool,
    /// The `Data` factor, or `px_per_unit` in logical pixels.
    pub(crate) scale: f32,
    /// `VECTOR_PIVOT_*`.
    pub(crate) pivot: u32,
    pub(crate) width_px: f32,
    pub(crate) head_length_px: f32,
    pub(crate) head_width_px: f32,
    pub(crate) min_length_px: f32,
}

/// What the chart's draw style draws for markers and lines, as the pick
/// shader's `styled` modes: the precise shapes and quads, the sketch wobble,
/// Milkyway planets and ribbons, or Constellation stars over precise lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GpuPickDrawStyle {
    marker: u32,
    line: u32,
}

impl GpuPickDrawStyle {
    pub(crate) const PRECISE: Self = Self {
        marker: PICK_MARKER_PRECISE,
        line: PICK_LINE_PRECISE,
    };

    /// The marker mode, as the scatter shader's picked ring reads it
    /// (`RING_MARKER_*`, numbered like `PICK_MARKER_*`).
    pub(crate) fn marker_mode(self) -> u32 {
        self.marker
    }

    pub(crate) fn of(style: &crate::config::DrawStyle) -> Self {
        use crate::config::DrawStyle;
        match style {
            DrawStyle::Precise => Self::PRECISE,
            DrawStyle::Sketch(_) => Self {
                marker: PICK_MARKER_SKETCH,
                line: PICK_LINE_SKETCH,
            },
            DrawStyle::Milkyway(_) => Self {
                marker: PICK_MARKER_PLANET,
                line: PICK_LINE_RIBBON,
            },
            DrawStyle::Constellation(_) => Self {
                marker: PICK_MARKER_STAR,
                line: PICK_LINE_PRECISE,
            },
        }
    }
}

/// Query state which integration can derive directly from `Config`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GpuPickQuery {
    pub(crate) transform: ScatterTransform,
    /// `(x, y, width, height)` in canvas pixels.  Width/height must be the
    /// same chart-area values used to build `transform`.
    pub(crate) chart_rect_px: [f32; 4],
    /// Current data-area clip in canvas pixels.  Like the CPU picker, cursor
    /// positions outside it return `None` before GPU work is submitted.
    pub(crate) data_area_px: Option<[f32; 4]>,
    pub(crate) canvas_position_px: [f32; 2],
    pub(crate) max_distance_px: f32,
    /// The draw style the chart is drawn with.
    pub(crate) draw_style: GpuPickDrawStyle,
}

/// Handles address only the currently uploaded bounded pair buffer.
#[derive(Clone, Copy)]
pub(crate) struct GpuStreamPickColumns {
    pub(crate) x: ColumnHandle,
    pub(crate) y: ColumnHandle,
    pub(crate) style_index: Option<ColumnHandle>,
    /// The `point_size_map` value column, resident series only.
    pub(crate) size_value: Option<ColumnHandle>,
    /// A point vector's `u`, `v` and colour columns, resident series only.
    pub(crate) vector: [Option<ColumnHandle>; 3],
}

pub(crate) fn validate_stream_handle(
    buffer: &wgpu::Buffer,
    handle: ColumnHandle,
) -> Result<(), GpuPickError> {
    let end = (handle.len_values as u64)
        .checked_mul(8)
        .and_then(|bytes| handle.offset.checked_add(bytes));
    if handle.offset % 8 != 0 || end.is_none_or(|end| end > buffer.size()) {
        return Err(GpuPickError::DeviceLimit {
            resource: "pick column pair range",
            requested: end.unwrap_or(u64::MAX),
            limit: buffer.size(),
        });
    }
    Ok(())
}

/// Constant-size running GPU answer. Source bytes remain owned by the stream
/// upload ring; every encoded chunk is reduced before that slot is reusable.
pub(crate) struct GpuStreamPick {
    engine: GpuPickEngine,
    /// The exact pipeline every chunk scans with: the marker-and-line module
    /// with every path (`exact_variant::ALL`); streaming is not specialised.
    exact: wgpu::ComputePipeline,
    query: GpuPickQuery,
    display_scale: f32,
    identities: Arc<[PickIdentity]>,
    has_chunks: bool,
    enabled: bool,
}

impl PickPipelineBundle {
    pub(crate) fn begin_stream(
        self: &Arc<Self>,
        query: GpuPickQuery,
        display_scale: f32,
        identities: Vec<(Option<String>, String)>,
        exact: wgpu::ComputePipeline,
    ) -> Result<GpuStreamPick, GpuPickError> {
        let reduce = GpuPickEngine::create_reduce_resources(
            &self.device,
            &self.ledger,
            &self.reduce_bgl,
            2,
        )?;
        let [cx, cy] = query.canvas_position_px;
        let [x, y, w, h] = query.chart_rect_px;
        let enabled = display_scale.is_finite()
            && display_scale > 0.0
            && [cx, cy, x, y, w, h, query.max_distance_px]
                .into_iter()
                .all(f32::is_finite)
            && w > 0.0
            && h > 0.0
            && query.max_distance_px >= 0.0
            && query.data_area_px.is_none_or(|[x, y, w, h]| {
                [x, y, w, h].into_iter().all(f32::is_finite)
                    && cx >= x
                    && cx <= x + w
                    && cy >= y
                    && cy <= y + h
            });
        Ok(GpuStreamPick {
            engine: GpuPickEngine::from_bundle_and_reduce(Arc::clone(self), reduce),
            exact,
            query,
            display_scale,
            identities: identities
                .into_iter()
                .map(|(source_id, series_id)| PickIdentity {
                    source_id,
                    series_id,
                })
                .collect(),
            has_chunks: false,
            enabled,
        })
    }
}

impl GpuStreamPick {
    pub(crate) const PERSISTENT_BYTES: u64 = 3 * CANDIDATE_BYTES;
    pub(crate) const READBACK_BYTES: u64 = CANDIDATE_BYTES;

    /// Exact chunk scratch allocation, excluding the caller-owned uploaded
    /// pair buffer and the constant running accumulator.
    pub(crate) fn chunk_bytes(
        &self,
        descriptor: &GpuPickSeriesDescriptor<'_>,
        paired_count: usize,
    ) -> Result<u64, GpuPickError> {
        let count = u32::try_from(paired_count).map_err(|_| GpuPickError::TooManyValues {
            series_id: descriptor.series_id.clone(),
            count: paired_count,
        })?;
        if count == 0 || !self.enabled {
            return Ok(0);
        }
        let (words, workgroups, _, _) = gate_dispatch_layout(
            count,
            self.engine
                .bundle
                .device
                .limits()
                .max_compute_workgroups_per_dimension,
        )?;
        let map = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.style_map.as_ref());
        let slots = map.map_or(1, |map| map.style_slots.len().max(1)) as u64;
        let overrides = map.map_or(1, |map| map.style_overrides.len().max(1)) as u64;
        let gate_words = if count <= GPU_PICK_DIRECT_SCAN_POINTS {
            1
        } else {
            words
        };
        u64::from(gate_words)
            .checked_mul(GATE_MASK_BYTES)
            .and_then(|bytes| bytes.checked_add(u64::from(workgroups) * CANDIDATE_BYTES))
            .and_then(|bytes| {
                bytes
                    .checked_add(std::mem::size_of::<PickQueryParamsGpu>() as u64 + CANDIDATE_BYTES)
            })
            .and_then(|bytes| {
                slots
                    .checked_mul(std::mem::size_of::<ScatterStyleSlotGpu>() as u64)
                    .and_then(|slots| bytes.checked_add(slots))
            })
            .and_then(|bytes| {
                overrides
                    .checked_mul(std::mem::size_of::<ScatterStyleOverrideGpu>() as u64)
                    .and_then(|overrides| bytes.checked_add(overrides))
            })
            .ok_or(GpuPickError::AllocationFailed {
                resource: "stream pick chunk scratch bytes",
            })
    }

    /// The caller retains this charge until the final referencing submission
    /// completes, including cancellation with recorded commands still alive.
    #[cfg(test)]
    pub(crate) fn charge(&self) -> SharedCharge {
        self.engine.registry.reduce._charge.clone()
    }

    pub(crate) fn result_buffer(&self) -> Option<&wgpu::Buffer> {
        self.has_chunks
            .then_some(&self.engine.registry.reduce.final_result)
    }

    /// All owned indices begin at local zero. A line chunk includes one right
    /// halo point; `owned_points` excludes that halo and `owned_segments` names
    /// exactly the starts owned by this chunk. Style override rows keep global
    /// indices, whereas the style-index handle addresses this chunk's lane.
    /// Returned scratch charge must live through the chunk's GPU completion.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_chunk(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        global_point_start: u32,
        owned_points: u32,
        owned_segments: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        self.encode_chunk_impl(
            encoder,
            buffer,
            descriptor,
            columns,
            series_order,
            global_point_start,
            owned_points,
            owned_segments,
            0,
        )
    }

    /// The final u32 lane of the same packed GPU buffer maps local row slots
    /// to original source rows. No source replay or CPU row mirror is needed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_indexed_chunk(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        original_index_word_base: u32,
        owned_points: u32,
        owned_segments: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        self.encode_chunk_impl(
            encoder,
            buffer,
            descriptor,
            columns,
            series_order,
            0,
            owned_points,
            owned_segments,
            original_index_word_base,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn encode_chunk_impl(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        series_order: u32,
        global_point_start: u32,
        owned_points: u32,
        owned_segments: u32,
        original_index_word_base: u32,
    ) -> Result<Option<SharedCharge>, GpuPickError> {
        if !self.enabled {
            return Ok(None);
        }
        let identity =
            self.identities
                .get(series_order as usize)
                .ok_or(GpuPickError::InvalidSeriesIndex {
                    index: series_order as usize,
                    len: self.identities.len(),
                })?;
        if identity.series_id != descriptor.series_id || identity.source_id != descriptor.source_id
        {
            return Err(GpuPickError::InvalidGpuResult);
        }
        let paired = columns.x.len_values.min(columns.y.len_values);
        if owned_points as usize > paired
            || owned_segments as usize > paired.saturating_sub(1)
            || global_point_start
                .checked_add(u32::try_from(paired.saturating_sub(1)).unwrap_or(u32::MAX))
                .is_none()
        {
            return Err(GpuPickError::TooManyValues {
                series_id: descriptor.series_id,
                count: paired,
            });
        }
        if owned_points == 0 && owned_segments == 0 {
            return Ok(None);
        }
        if original_index_word_base != 0 {
            let required = u64::from(original_index_word_base)
                .checked_add(paired as u64)
                .and_then(|words| words.checked_mul(4))
                .ok_or(GpuPickError::InvalidGpuResult)?;
            if required > buffer.size() {
                return Err(GpuPickError::InvalidGpuResult);
            }
        }
        let slot = self
            .engine
            .prepare_series_buffer(buffer, descriptor, columns, None)?;
        let series = &slot.gpu;
        // Streaming is not extended: a streamed line is picked without its
        // arc prefix, so its dash gaps pick like its dashes.
        let params = query_params(
            series,
            &slot.identity.series_id,
            &self.query,
            self.display_scale,
            series_order,
            [
                series.point_count,
                lane_base(columns.x, &slot.identity.series_id)?,
                lane_base(columns.y, &slot.identity.series_id)?,
                series.style_index_base,
            ],
            [
                global_point_start,
                owned_points,
                owned_segments,
                original_index_word_base,
            ],
            false,
        );
        self.engine
            .bundle
            .queue
            .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        let bundle = &self.engine.bundle;
        for (pipeline, enabled) in [
            (&bundle.gate_x, !series.direct_scan),
            (&bundle.gate_y, !series.direct_scan),
            (&self.exact, true),
        ] {
            if !enabled {
                continue;
            }
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick chunk scan"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &series.query_data_bg, &[]);
            pass.set_bind_group(1, &series.query_work_bg, &[]);
            pass.set_bind_group(2, &bundle.arc_placeholder, &[]);
            pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick chunk reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bundle.reduce);
            pass.set_bind_group(0, &series.reduce_bg, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        let reduce = &self.engine.registry.reduce;
        if self.has_chunks {
            encoder.copy_buffer_to_buffer(
                &reduce.final_result,
                0,
                &reduce.candidates,
                0,
                CANDIDATE_BYTES,
            );
        } else {
            encoder.clear_buffer(&reduce.candidates, 0, None);
        }
        encoder.copy_buffer_to_buffer(
            &series.result,
            0,
            &reduce.candidates,
            CANDIDATE_BYTES,
            CANDIDATE_BYTES,
        );
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy stream pick running reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&bundle.reduce);
            pass.set_bind_group(0, &reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        self.has_chunks = true;
        Ok(Some(series._charge.clone()))
    }

    /// Call only after every required chunk has been encoded and submitted.
    pub(crate) fn finish(self) -> Result<GpuPickTicket, GpuPickError> {
        if !self.has_chunks {
            return Ok(GpuPickTicket::ready_none());
        }
        let bundle = &self.engine.bundle;
        let tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &tally,
            &bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy stream pick scalar readback"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "stream pick readback",
        )?;
        let mut encoder = bundle
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("figgy stream pick finish"),
            });
        encoder.copy_buffer_to_buffer(
            &self.engine.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        bundle.queue.submit([encoder.finish()]);
        let (sender, receiver) = oneshot::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = sender.send(result);
            });
        Ok(GpuPickTicket {
            state: GpuPickTicketState::Pending {
                device: Arc::clone(&bundle.device),
                readback,
                receiver,
                identities: self.identities,
                _charge: tally.into_charge(&bundle.ledger, GpuResourceKind::Readback),
                _scratch_charge: Some(self.engine.registry.reduce._charge.clone()),
            },
        })
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct GpuPickSeriesId(u32);

#[cfg(test)]
impl GpuPickSeriesId {
    fn index(self) -> u32 {
        self.0
    }
}

/// `CONSTELLATION_POINT_STAR_GAIN` times the largest per-point size jitter
/// of `constellation_star_radius` (`0.86 + 0.30`).
const CONSTELLATION_STAR_MAX_SCALE: f32 = 1.16;
/// `PLANET_QUAD_EXTENT`: a planet's quad half-extent per unit radius.
const PLANET_QUAD_EXTENT: f32 = 2.5;

/// The pick shader's per-query uniform for one registered series. `data` is
/// `(point count, x lane base, y lane base, style-index lane base)` and
/// `stream` the `(global offset, owned scatter count, owned segment count,
/// row-map base)` of the chunk; a resident series owns every point. `arc`
/// says whether the exact pass binds this series' arc prefix.
#[allow(clippy::too_many_arguments)]
fn query_params(
    series: &PickSeriesGpu,
    series_id: &str,
    query: &GpuPickQuery,
    display_scale: f32,
    series_order: u32,
    data: [u32; 4],
    stream: [u32; 4],
    arc: bool,
) -> PickQueryParamsGpu {
    let [cursor_x, cursor_y] = query.canvas_position_px;
    let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
    let style = query.draw_style;
    let params = query.transform.style_params;
    // Nothing a marker draws lies outside its quad: the visual radius plus
    // the quad margin on either axis, grown by the styled entry's own quad.
    let base_radius = series.base_radius_px * display_scale;
    let marker_reach = match style.marker {
        PICK_MARKER_SKETCH => {
            series.max_extent_px * display_scale
                + MARKER_QUAD_MARGIN_PX
                + params[0][0].max(0.0) * 0.5
        }
        PICK_MARKER_PLANET => base_radius.max(1.0) * PLANET_QUAD_EXTENT + MARKER_QUAD_MARGIN_PX,
        PICK_MARKER_STAR => {
            (base_radius * CONSTELLATION_STAR_MAX_SCALE).max(0.5) * 4.0 + MARKER_QUAD_MARGIN_PX
        }
        _ => series.max_extent_px * display_scale + MARKER_QUAD_MARGIN_PX,
    };
    // The line shader's `max(line_width_px, 1) / 2`.
    let half_width = (2.0 * series.line_half_width_px * display_scale).max(1.0) * 0.5;
    let ribbon = milkyway_line_half_extent(&params);
    let line_extent = match style.line {
        PICK_LINE_SKETCH => half_width + crate::pick::MARKER_STROKE_HALF_PX + params[0][0].max(0.0),
        PICK_LINE_RIBBON => ribbon,
        _ => half_width + crate::pick::MARKER_STROKE_HALF_PX,
    };
    let mut line_dash = [[0.0f32; 4]; 2];
    for (index, length) in series.line_dash.iter().enumerate() {
        line_dash[index / 4][index % 4] = length * display_scale;
    }
    // A milkyway ribbon is never dashed.
    let dash_count = if style.line == PICK_LINE_RIBBON {
        0
    } else {
        series.line_dash_len
    };
    PickQueryParamsGpu {
        transform: query.transform,
        cursor_chart: [cursor_x, cursor_y, chart_x, chart_y],
        chart_limits: [
            chart_width,
            chart_height,
            query.max_distance_px,
            marker_reach,
        ],
        scatter_line: [
            series.base_radius_px,
            series.line_half_width_px,
            display_scale,
            if series.direct_scan { 1.0 } else { 0.0 },
        ],
        data,
        style: [
            series.flags,
            series.style_count,
            series.override_count,
            series.style_index_len,
        ],
        series: [
            series.gate_word_count,
            series_order,
            series.base_shape_id,
            series.dispatch_x,
        ],
        stream,
        line: [series.line_step_mode, u32::from(arc), 0, 0],
        size_map: [series.size_base, series.size_len, 0, 0],
        size_domain: series.size_domain,
        size_px: [series.size_px[0], series.size_px[1], 0.0, 0.0],
        // A square cap's corner reaches √2 times the half extent past the
        // centreline on either axis.
        line_px: [
            half_width,
            ribbon,
            line_extent * std::f32::consts::SQRT_2,
            0.0,
        ],
        line_dash,
        styled: [
            style.marker,
            style.line,
            crate::sketch::fnv1a(series_id),
            dash_count,
        ],
        vector: [
            series.vector_base[0],
            series.vector_base[1],
            series.vector_count,
            series.vector.as_ref().map_or(0, |vector| {
                vector.pivot
                    | if vector.scale_pixels {
                        VECTOR_SCALE_PIXELS
                    } else {
                        0
                    }
            }),
        ],
        vector_px: series.vector.as_ref().map_or([0.0; 4], |vector| {
            [
                if vector.scale_pixels {
                    vector.scale * display_scale
                } else {
                    vector.scale
                },
                vector.width_px * display_scale,
                vector.head_length_px * display_scale,
                vector.head_width_px * display_scale,
            ]
        }),
        vector_min: [
            series
                .vector
                .as_ref()
                .map_or(0.0, |vector| vector.min_length_px * display_scale),
            0.0,
            0.0,
            0.0,
        ],
    }
}

/// The half extent of what a Milkyway line draws around its centreline, in
/// canvas pixels, from the packed style parameters (`pack_milkyway_params`):
/// the ribbon strip reaches one full ribbon width either side
/// (`vs_ribbon`'s `max(ribbon_width_px, 1)`), and a star of the arc-driven
/// star pass (`vs_stars`) lies within its perpendicular scatter
/// (`1.7 * spread_px`), a binary companion's separation (`4 *
/// structure_scale`) and its quad (`4 * star_scale * 4.3` at full
/// brightness) of the path. The star positions are hashed per slot, so this
/// envelope, not each star, is what picks.
fn milkyway_line_half_extent(params: &[[f32; 4]; 3]) -> f32 {
    let ribbon = params[0][1].max(1.0);
    let star_scale = params[1][0].max(0.0);
    let spread = params[1][1].max(0.0);
    let structure = params[2][0].max(1e-3);
    let stars = if params[0][0] > 0.0 {
        spread * 1.7 + 4.0 * structure + star_scale * (0.9 + 3.4) * 4.0
    } else {
        0.0
    };
    ribbon.max(stars)
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PickQueryParamsGpu {
    transform: ScatterTransform,
    cursor_chart: [f32; 4],
    chart_limits: [f32; 4],
    scatter_line: [f32; 4],
    data: [u32; 4],
    style: [u32; 4],
    series: [u32; 4],
    stream: [u32; 4],
    line: [u32; 4],
    /// `(size lane base, size logical length, 0, 0)`.
    size_map: [u32; 4],
    /// `(domain_min hi, domain_min lo, domain_max hi, domain_max lo)`.
    size_domain: [f32; 4],
    /// `(min_px, max_px, 0, 0)`, logical pixels.
    size_px: [f32; 4],
    /// `(stroke half-width, milkyway half extent, line gate reach, 0)`,
    /// canvas pixels.
    line_px: [f32; 4],
    /// The dash pattern in canvas pixels.
    line_dash: [[f32; 4]; 2],
    /// `(marker mode, line mode, series salt, dash count)`.
    styled: [u32; 4],
    /// `(u lane base, v lane base, arrow count, pivot | scale-pixels bit)`.
    vector: [u32; 4],
    /// `(scale, shaft width, head length, head width)`, canvas pixels.
    vector_px: [f32; 4],
    /// `(minimum length, 0, 0, 0)`, canvas pixels.
    vector_min: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct PickCandidateGpu {
    valid: u32,
    series_order: u32,
    point_index: u32,
    primitive_kind: u32,
    primitive_index: u32,
    distance_sq: f32,
    distance_px: f32,
    _pad: u32,
}

const _: () = assert!(std::mem::size_of::<PickQueryParamsGpu>() == 432);
const _: () = assert!(std::mem::size_of::<PickCandidateGpu>() == CANDIDATE_BYTES as usize);

#[derive(Clone)]
struct PickIdentity {
    source_id: Option<String>,
    series_id: String,
}

/// Immutable device resources shared by every picker registry on one renderer.
pub(crate) struct PickPipelineBundle {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    /// Where pick scratch is charged. Carried by the bundle so every
    /// `&self` query path can charge without a per-call argument.
    ledger: Arc<GpuLedger>,
    query_data_bgl: wgpu::BindGroupLayout,
    query_work_bgl: wgpu::BindGroupLayout,
    reduce_bgl: wgpu::BindGroupLayout,
    gate_x: wgpu::ComputePipeline,
    gate_y: wgpu::ComputePipeline,
    /// The layout every exact pipeline uses ([`PickExactPipelines`]).
    exact_layout: wgpu::PipelineLayout,
    reduce: wgpu::ComputePipeline,
    /// The exact pass's group 2: one series' arc prefix (`pick_arc`).
    arc_bgl: wgpu::BindGroupLayout,
    /// Group 2 for a series picked without an arc prefix: one zero.
    arc_placeholder: wgpu::BindGroup,
    _arc_placeholder_charge: SharedCharge,
}

#[derive(Clone)]
struct PickSeriesGpu {
    /// Charge for this series' gate masks, workgroup candidates, params, style
    /// rows, and scalar result. Shared across clones like `ReduceResources`.
    _charge: SharedCharge,
    pool_identity: Option<PoolIdentity>,
    x_column: ColumnId,
    y_column: ColumnId,
    style_index_column: Option<ColumnId>,
    x_handle: ColumnHandle,
    y_handle: ColumnHandle,
    style_index_handle: Option<ColumnHandle>,
    pool_layout_generation: u64,
    x_allocation_epoch: u64,
    y_allocation_epoch: u64,
    style_index_allocation_epoch: Option<u64>,
    size_column: Option<ColumnId>,
    size_handle: Option<ColumnHandle>,
    size_allocation_epoch: Option<u64>,
    /// A point vector's arrows: the style, the `u` / `v` / colour columns
    /// with their handles and allocation epochs, the component lane bases
    /// and the arrow count.
    vector: Option<GpuPickVector>,
    vector_handles: [Option<ColumnHandle>; 3],
    vector_epochs: [Option<u64>; 3],
    vector_base: [u32; 2],
    vector_count: u32,
    size_base: u32,
    size_len: u32,
    size_domain: [f32; 4],
    size_px: [f32; 2],
    query_params: wgpu::Buffer,
    gate_masks: wgpu::Buffer,
    workgroup_candidates: wgpu::Buffer,
    query_data_bg: wgpu::BindGroup,
    query_work_bg: wgpu::BindGroup,
    reduce_bg: wgpu::BindGroup,
    result: wgpu::Buffer,
    point_count: u32,
    gate_word_count: u32,
    dispatch_x: u32,
    dispatch_y: u32,
    direct_scan: bool,
    flags: u32,
    base_radius_px: f32,
    base_shape_id: u32,
    line_half_width_px: f32,
    line_step_mode: u32,
    /// The dash pattern in logical pixels and how many of its lengths count.
    line_dash: [f32; 8],
    line_dash_len: u32,
    max_extent_px: f32,
    style_count: u32,
    override_count: u32,
    style_index_base: u32,
    style_index_len: u32,
}

#[derive(Clone)]
struct PickSeriesSlot {
    identity: PickIdentity,
    gpu: PickSeriesGpu,
}

#[derive(Clone)]
struct ReduceResources {
    candidates: wgpu::Buffer,
    final_result: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    capacity: usize,
    /// Charge for `candidates` + `final_result`. Shared because this struct is
    /// cloned and the clones hold the same device buffers — see
    /// [`SharedCharge`].
    _charge: SharedCharge,
}

struct PickRegistry {
    slots: Vec<PickSeriesSlot>,
    identities: Arc<[PickIdentity]>,
    reduce: ReduceResources,
    pool_identity: Option<PoolIdentity>,
    pool_layout_generation: u64,
    generation: u64,
}

/// Persistent exact GPU picking engine.
pub(crate) struct GpuPickEngine {
    bundle: Arc<PickPipelineBundle>,
    registry: PickRegistry,
    #[cfg(test)]
    fail_next_registry_prepare: std::cell::Cell<bool>,
}

/// One slot in a complete successor registry.
pub(crate) enum GpuPickRegistrySlot<'a> {
    /// Reuse GPU resources from `current_index`, optionally with new identity
    /// strings. The target pool is still validated and relocation-rebound.
    Reuse {
        current_index: usize,
        source_id: Option<String>,
        series_id: String,
    },
    Build(GpuPickSeriesDescriptor<'a>),
}

/// Engine-bound, fully prepared registry successor.
///
/// Its exclusive borrow makes wrong-engine and stale-generation commits
/// unrepresentable. Dropping it leaves the engine unchanged; `commit` only
/// moves the already-complete successor state.
#[must_use = "a prepared GPU-pick transition has no effect until it is committed"]
pub(crate) struct PreparedPickRegistryTransition<'engine> {
    engine: &'engine mut GpuPickEngine,
    next: Option<PickRegistry>,
}

impl PreparedPickRegistryTransition<'_> {
    pub(crate) fn commit(mut self) {
        self.engine.registry = self
            .next
            .take()
            .expect("prepared GPU-pick transition always owns a successor");
    }
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

// Renderer GPU ownership is Arc-based on every target. WebGPU handles are
// intentionally !Send/!Sync on wasm because JavaScript confines them locally.
#[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
fn create_pipeline_bundle_observed(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    observer: &mut dyn FnMut(InitEvent),
) -> Arc<PickPipelineBundle> {
    started(observer, INIT_SCOPE, "setup");
    // The gate module only: the exact modules compile on first use
    // (`PickExactPipelines`), the variant a series needs.
    let shader = crate::gpu_compile::shader_module(
        &device,
        wgpu::ShaderModuleDescriptor {
            label: Some(PickShader::Gate.label()),
            source: wgpu::ShaderSource::Wgsl(PickShader::Gate.source().into()),
        },
    );

    let query_data_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query data bgl"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, false),
                storage_entry(2, false),
            ],
        },
    );
    let query_work_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query work bgl"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        },
    );
    let reduce_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick reduction bgl"),
            entries: &[storage_entry(3, true), storage_entry(4, false)],
        },
    );

    let query_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick query layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl)],
            immediate_size: 0,
        },
    );
    let arc_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick arc prefix bgl"),
            entries: &[storage_entry(0, true)],
        },
    );
    let exact_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick exact layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl), Some(&arc_bgl)],
            immediate_size: 0,
        },
    );
    let (arc_placeholder, arc_placeholder_charge) =
        create_arc_placeholder(&device, &ledger, &arc_bgl);
    let reduce_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick reduction layout"),
            bind_group_layouts: &[Some(&reduce_bgl)],
            immediate_size: 0,
        },
    );
    finished(observer, INIT_SCOPE, "setup");
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, label: &'static str| {
        crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: if entry == "pick_reduce_candidates" {
                    crate::gpu_compile::workgroup_options(&device)
                } else {
                    wgpu::PipelineCompilationOptions::default()
                },
                cache: None,
            },
        )
    };

    Arc::new(PickPipelineBundle {
        ledger,
        gate_x: observe_value(observer, INIT_SCOPE, "pick_gate_x", || {
            pipeline(
                &query_layout,
                "pick_gate_x",
                "figgy GPU pick X gate pipeline",
            )
        }),
        gate_y: observe_value(observer, INIT_SCOPE, "pick_gate_y", || {
            pipeline(
                &query_layout,
                "pick_gate_y",
                "figgy GPU pick Y gate pipeline",
            )
        }),
        exact_layout,
        reduce: observe_value(observer, INIT_SCOPE, "pick_reduce_candidates", || {
            pipeline(
                &reduce_layout,
                "pick_reduce_candidates",
                "figgy GPU pick candidate reduction pipeline",
            )
        }),
        query_data_bgl,
        query_work_bgl,
        reduce_bgl,
        arc_bgl,
        arc_placeholder,
        _arc_placeholder_charge: arc_placeholder_charge,
        device,
        queue,
    })
}

/// The exact pass's group 2 for a series without an arc prefix: a one-value
/// storage buffer the shader never reads (`line.y` is zero).
fn create_arc_placeholder(
    device: &wgpu::Device,
    ledger: &Arc<GpuLedger>,
    arc_bgl: &wgpu::BindGroupLayout,
) -> (wgpu::BindGroup, SharedCharge) {
    let tally = ChargeTally::new();
    // gpu-alloc: PickScratch
    let buffer = charged_buffer_init(
        &tally,
        device,
        &wgpu::util::BufferInitDescriptor {
            label: Some("figgy GPU pick arc placeholder"),
            contents: bytemuck::bytes_of(&0.0f32),
            usage: wgpu::BufferUsages::STORAGE,
        },
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("figgy GPU pick arc placeholder bg"),
        layout: arc_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    (
        bind_group,
        shared_charge(tally, ledger, GpuResourceKind::PickScratch),
    )
}

impl PickPipelineBundle {
    /// The exact pass's group 2 holding one series' arc prefix.
    fn arc_bind_group(&self, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("figgy GPU pick arc prefix bg"),
            layout: &self.arc_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }
}

#[cfg_attr(target_arch = "wasm32", allow(clippy::arc_with_non_send_sync))]
async fn create_pipeline_bundle_observed_async(
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    ledger: Arc<GpuLedger>,
    observer: &mut dyn FnMut(InitEvent),
) -> Result<Arc<PickPipelineBundle>, GpuPickError> {
    use crate::init::{observe_value_async, yield_init_frame};
    #[cfg(target_arch = "wasm32")]
    crate::init::prewarm_compute_entries_js(
        &device,
        "gpu.pick.async",
        PickShader::Gate.source(),
        &[
            ("pick_gate_x", "pick_gate_x"),
            ("pick_gate_y", "pick_gate_y"),
            ("pick_reduce_candidates", "pick_reduce_candidates"),
        ],
        observer,
    )
    .await
    .map_err(GpuPickError::AsyncCompileFailed)?;
    started(observer, INIT_SCOPE, "setup");
    // The gate module only: the exact modules compile on first use
    // (`PickExactPipelines`), the variant a series needs.
    let shader = crate::gpu_compile::shader_module(
        &device,
        wgpu::ShaderModuleDescriptor {
            label: Some(PickShader::Gate.label()),
            source: wgpu::ShaderSource::Wgsl(PickShader::Gate.source().into()),
        },
    );

    let query_data_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query data bgl"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, false),
                storage_entry(2, false),
            ],
        },
    );
    let query_work_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick query work bgl"),
            entries: &[
                uniform_entry(0),
                storage_entry(1, true),
                storage_entry(2, true),
                storage_entry(3, false),
            ],
        },
    );
    let reduce_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick reduction bgl"),
            entries: &[storage_entry(3, true), storage_entry(4, false)],
        },
    );

    let query_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick query layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl)],
            immediate_size: 0,
        },
    );
    let arc_bgl = crate::gpu_compile::bind_group_layout(
        &device,
        &wgpu::BindGroupLayoutDescriptor {
            label: Some("figgy GPU pick arc prefix bgl"),
            entries: &[storage_entry(0, true)],
        },
    );
    let exact_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick exact layout"),
            bind_group_layouts: &[Some(&query_data_bgl), Some(&query_work_bgl), Some(&arc_bgl)],
            immediate_size: 0,
        },
    );
    let (arc_placeholder, arc_placeholder_charge) =
        create_arc_placeholder(&device, &ledger, &arc_bgl);
    let reduce_layout = crate::gpu_compile::pipeline_layout(
        &device,
        &wgpu::PipelineLayoutDescriptor {
            label: Some("figgy GPU pick reduction layout"),
            bind_group_layouts: &[Some(&reduce_bgl)],
            immediate_size: 0,
        },
    );
    finished(observer, INIT_SCOPE, "setup");
    yield_init_frame().await;
    let pipeline = |layout: &wgpu::PipelineLayout, entry: &str, label: &'static str| {
        crate::gpu_compile::compute_pipeline(
            &device,
            &wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                module: &shader,
                entry_point: Some(entry),
                compilation_options: if entry == "pick_reduce_candidates" {
                    crate::gpu_compile::workgroup_options(&device)
                } else {
                    wgpu::PipelineCompilationOptions::default()
                },
                cache: None,
            },
        )
    };

    let gate_x = observe_value_async(observer, INIT_SCOPE, "pick_gate_x", || {
        pipeline(
            &query_layout,
            "pick_gate_x",
            "figgy GPU pick X gate pipeline",
        )
    })
    .await;
    let gate_y = observe_value_async(observer, INIT_SCOPE, "pick_gate_y", || {
        pipeline(
            &query_layout,
            "pick_gate_y",
            "figgy GPU pick Y gate pipeline",
        )
    })
    .await;
    let reduce = observe_value_async(observer, INIT_SCOPE, "pick_reduce_candidates", || {
        pipeline(
            &reduce_layout,
            "pick_reduce_candidates",
            "figgy GPU pick candidate reduction pipeline",
        )
    })
    .await;

    Ok(Arc::new(PickPipelineBundle {
        gate_x,
        gate_y,
        exact_layout,
        reduce,
        query_data_bgl,
        query_work_bgl,
        reduce_bgl,
        arc_bgl,
        arc_placeholder,
        _arc_placeholder_charge: arc_placeholder_charge,
        device,
        queue,
        ledger,
    }))
}

/// Every pick buffer goes through here, and the tally is not optional, so a new
/// pick allocation cannot skip the ledger.
fn create_buffer_checked(
    tally: &ChargeTally,
    device: &wgpu::Device,
    desc: &wgpu::BufferDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    // gpu-alloc: PickScratch
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        charged_buffer(tally, device, desc)
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

fn create_buffer_init_checked(
    tally: &ChargeTally,
    device: &wgpu::Device,
    label: &'static str,
    contents: &[u8],
    usage: wgpu::BufferUsages,
    resource: &'static str,
) -> Result<wgpu::Buffer, GpuPickError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // gpu-alloc: PickScratch
        charged_buffer_init(
            tally,
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            },
        )
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

fn create_bind_group_checked(
    device: &wgpu::Device,
    desc: &wgpu::BindGroupDescriptor<'_>,
    resource: &'static str,
) -> Result<wgpu::BindGroup, GpuPickError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        device.create_bind_group(desc)
    }))
    .map_err(|_| GpuPickError::AllocationFailed { resource })
}

fn nonnegative(v: f32) -> f32 {
    if !v.is_nan() && v > 0.0 { v } else { 0.0 }
}

/// The factor `visual_shape_radius` gives shape `shape_id` (SHADER_COMMON.md
/// §5.5): its drawn radius per unit of reference radius.
fn shape_scale(shape_id: u32) -> f32 {
    crate::pick::marker_visual_scale(crate::pick::marker_base_shape_id(shape_id))
}

fn gpu_meta_u32(v: f32) -> Option<u32> {
    if !v.is_finite() || !(0.0..4_294_967_296.0).contains(&v) || v.fract() != 0.0 {
        None
    } else {
        Some(v as u32)
    }
}

fn style_max_extent(series_id: &str, scatter: &GpuPickScatter<'_>) -> Result<f32, GpuPickError> {
    let mut maximum_radius = nonnegative(scatter.base_radius_px);
    let mut maximum_scale = shape_scale(scatter.base_shape_id);
    // A size map clamps to its pixel range, so `max_px` bounds every mapped
    // radius; an override row may still exceed it and is visited below.
    if let Some(size_map) = scatter.size_map.as_ref() {
        maximum_radius = maximum_radius.max(nonnegative(size_map.max_px));
    }
    let Some(map) = scatter.style_map.as_ref() else {
        return Ok(maximum_radius * maximum_scale);
    };

    if map.style_meta.style_count as usize > map.style_slots.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "style_count exceeds the supplied slot array",
        });
    }
    if map.style_meta.override_count as usize > map.style_overrides.len() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "override_count exceeds the supplied override array",
        });
    }
    let has_index = map.style_meta.has_index != 0;
    if map.style_meta.has_index > 1 || has_index != map.style_index_column.is_some() {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "has_index and style_index_column disagree",
        });
    }

    let mut visit = |meta: [f32; 4]| -> Result<(), GpuPickError> {
        let mask = gpu_meta_u32(meta[2]).ok_or_else(|| GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "a style mask is not a finite integer",
        })?;
        if mask & STYLE_MASK_RADIUS != 0 {
            maximum_radius = maximum_radius.max(nonnegative(meta[0]));
        }
        if mask & STYLE_MASK_SHAPE != 0 {
            let shape = gpu_meta_u32(meta[1]).ok_or_else(|| GpuPickError::InvalidStyleMap {
                series_id: series_id.to_owned(),
                reason: "a mapped shape id is not a finite integer",
            })?;
            maximum_scale = maximum_scale.max(shape_scale(shape));
        }
        Ok(())
    };
    for slot in &map.style_slots[..map.style_meta.style_count as usize] {
        visit(slot.meta)?;
    }
    for override_ in &map.style_overrides[..map.style_meta.override_count as usize] {
        visit(override_.meta)?;
    }

    // Radius and shape changes may originate in different table/override
    // rows. Their product is deliberately a cross-product upper bound; the
    // final exact pass still applies rows in production order.
    Ok(maximum_radius * maximum_scale)
}

fn lane_base(handle: ColumnHandle, series_id: &str) -> Result<u32, GpuPickError> {
    if !handle.offset.is_multiple_of(4) {
        return Err(GpuPickError::InvalidStyleMap {
            series_id: series_id.to_owned(),
            reason: "column byte offset is not f32-aligned",
        });
    }
    u32::try_from(handle.offset / 4).map_err(|_| GpuPickError::DeviceLimit {
        resource: "column word offset",
        requested: handle.offset / 4,
        limit: u64::from(u32::MAX),
    })
}

fn gate_dispatch_layout(
    point_count: u32,
    max_workgroups_per_dimension: u32,
) -> Result<(u32, u32, u32, u32), GpuPickError> {
    if max_workgroups_per_dimension == 0 {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_workgroups_per_dimension",
            requested: 1,
            limit: 0,
        });
    }
    let gate_word_count = point_count.div_ceil(GPU_PICK_GATE_WORD_POINTS);
    let workgroup_count = gate_word_count.div_ceil(GPU_PICK_WORKGROUP_SIZE);
    let dispatch_x = workgroup_count.min(max_workgroups_per_dimension);
    let dispatch_y = workgroup_count.div_ceil(dispatch_x);
    if dispatch_y > max_workgroups_per_dimension {
        return Err(GpuPickError::DeviceLimit {
            resource: "GPU pick gate dispatch",
            requested: u64::from(workgroup_count),
            limit: u64::from(max_workgroups_per_dimension)
                * u64::from(max_workgroups_per_dimension),
        });
    }
    Ok((gate_word_count, workgroup_count, dispatch_x, dispatch_y))
}

fn same_storage(a: ColumnHandle, b: ColumnHandle) -> bool {
    a.offset == b.offset && a.byte_size == b.byte_size && a.len_values == b.len_values
}

fn validate_device_limits(device: &wgpu::Device) -> Result<(), GpuPickError> {
    let limits = device.limits();
    if limits.max_compute_workgroup_size_x < GPU_PICK_WORKGROUP_SIZE {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_workgroup_size_x",
            requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
            limit: u64::from(limits.max_compute_workgroup_size_x),
        });
    }
    if limits.max_compute_invocations_per_workgroup < GPU_PICK_WORKGROUP_SIZE {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_compute_invocations_per_workgroup",
            requested: u64::from(GPU_PICK_WORKGROUP_SIZE),
            limit: u64::from(limits.max_compute_invocations_per_workgroup),
        });
    }
    if limits.max_bind_groups < 2 {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_bind_groups",
            requested: 2,
            limit: u64::from(limits.max_bind_groups),
        });
    }
    if limits.max_storage_buffers_per_shader_stage < 6 {
        return Err(GpuPickError::DeviceLimit {
            resource: "max_storage_buffers_per_shader_stage",
            requested: 6,
            limit: u64::from(limits.max_storage_buffers_per_shader_stage),
        });
    }
    Ok(())
}

#[allow(dead_code)] // Consumed by renderer ownership integration in P-02.
impl PickPipelineBundle {
    pub(crate) fn new_observed(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, GpuPickError> {
        validate_device_limits(&device)?;
        Ok(create_pipeline_bundle_observed(
            device, queue, ledger, observer,
        ))
    }

    pub(crate) async fn new_observed_async(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        ledger: Arc<GpuLedger>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Arc<Self>, GpuPickError> {
        validate_device_limits(&device)?;
        create_pipeline_bundle_observed_async(device, queue, ledger, observer).await
    }
}

impl GpuPickEngine {
    #[cfg(test)]
    fn new(device: Arc<wgpu::Device>, queue: Arc<wgpu::Queue>) -> Result<Self, GpuPickError> {
        let mut noop = |_| {};
        Self::new_observed(device, queue, &mut noop)
    }

    #[cfg(test)]
    fn new_observed(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        observer: &mut dyn FnMut(InitEvent),
    ) -> Result<Self, GpuPickError> {
        observe_result(observer, INIT_SCOPE, "limits", || {
            validate_device_limits(&device)
        })?;
        let bundle =
            create_pipeline_bundle_observed(device, queue, Arc::new(GpuLedger::new()), observer);
        let reduce = observe_result(observer, INIT_SCOPE, "initial_reduce_resources", || {
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)
        })?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    #[allow(dead_code)] // Consumed by renderer ownership integration in P-02.
    pub(crate) fn from_bundle(bundle: Arc<PickPipelineBundle>) -> Result<Self, GpuPickError> {
        let reduce =
            Self::create_reduce_resources(&bundle.device, &bundle.ledger, &bundle.reduce_bgl, 1)?;
        Ok(Self::from_bundle_and_reduce(bundle, reduce))
    }

    fn from_bundle_and_reduce(bundle: Arc<PickPipelineBundle>, reduce: ReduceResources) -> Self {
        Self {
            bundle,
            registry: PickRegistry {
                slots: Vec::new(),
                identities: Arc::from([]),
                reduce,
                pool_identity: None,
                pool_layout_generation: 0,
                generation: 0,
            },
            #[cfg(test)]
            fail_next_registry_prepare: std::cell::Cell::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn registry_generation(&self) -> u64 {
        self.registry.generation
    }

    #[cfg(test)]
    pub(crate) fn registered_series_count(&self) -> usize {
        self.registry.slots.len()
    }

    #[cfg(test)]
    pub(crate) fn query_params_buffer(&self, index: usize) -> Option<wgpu::Buffer> {
        self.registry
            .slots
            .get(index)
            .map(|slot| slot.gpu.query_params.clone())
    }

    #[cfg(test)]
    pub(crate) fn identity_snapshot(&self) -> Vec<(Option<String>, String)> {
        self.registry
            .identities
            .iter()
            .map(|identity| (identity.source_id.clone(), identity.series_id.clone()))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn fail_next_registry_prepare(&self) {
        self.fail_next_registry_prepare.set(true);
    }

    fn create_reduce_resources(
        device: &wgpu::Device,
        ledger: &Arc<GpuLedger>,
        layout: &wgpu::BindGroupLayout,
        candidate_count: usize,
    ) -> Result<ReduceResources, GpuPickError> {
        let capacity = candidate_count.max(1);
        let tally = ChargeTally::new();
        let count = capacity as u64;
        let candidate_size =
            count
                .checked_mul(CANDIDATE_BYTES)
                .ok_or(GpuPickError::DeviceLimit {
                    resource: "candidate buffer",
                    requested: u64::MAX,
                    limit: device.limits().max_buffer_size,
                })?;
        let candidates = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick per-series candidates"),
                size: candidate_size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "per-series candidate buffer",
        )?;
        let final_result = create_buffer_checked(
            &tally,
            device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick final scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "final pick scalar buffer",
        )?;
        let bind_group = create_bind_group_checked(
            device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick reduction bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: candidates.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: final_result.as_entire_binding(),
                    },
                ],
            },
            "reduction bind group",
        )?;
        Ok(ReduceResources {
            candidates,
            final_result,
            bind_group,
            capacity,
            _charge: shared_charge(tally, ledger, GpuResourceKind::PickScratch),
        })
    }

    fn prepare_reduce_resources(
        &self,
        candidate_count: usize,
    ) -> Result<ReduceResources, GpuPickError> {
        if candidate_count <= self.registry.reduce.capacity {
            Ok(self.registry.reduce.clone())
        } else {
            Self::create_reduce_resources(
                &self.bundle.device,
                &self.bundle.ledger,
                &self.bundle.reduce_bgl,
                candidate_count,
            )
        }
    }

    /// Prepare one persistent exact query slot from `pool`.
    ///
    /// Every later [`Self::pick`] must receive that same pool instance. Replace
    /// this slot after any referenced column content replacement. Allocation
    /// epochs distinguish remove+readd even when the replacement reuses the
    /// same offset and length.
    #[cfg(test)]
    fn add_series(
        &mut self,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let id = GpuPickSeriesId(self.registry.slots.len() as u32);
        let mut final_slots = self.current_reuse_slots(None);
        final_slots.push(GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(id)
    }

    fn prepare_series(
        &self,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<PickSeriesSlot, GpuPickError> {
        let x_handle = pool
            .handle_for(&descriptor.x_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.x_column.clone()))?;
        let y_handle = pool
            .handle_for(&descriptor.y_column)
            .ok_or_else(|| GpuPickError::MissingColumn(descriptor.y_column.clone()))?;
        let style_index_handle = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.style_map.as_ref())
            .and_then(|map| map.style_index_column.as_ref())
            .map(|id| {
                pool.handle_for(id)
                    .ok_or_else(|| GpuPickError::MissingColumn(id.clone()))
            })
            .transpose()?;
        let size_value_handle = descriptor
            .scatter
            .as_ref()
            .and_then(|scatter| scatter.size_map.as_ref())
            .map(|map| {
                pool.handle_for(&map.column)
                    .ok_or_else(|| GpuPickError::MissingColumn(map.column.clone()))
            })
            .transpose()?;
        let mut vector = [None; 3];
        if let Some(arrows) = descriptor.vector.as_ref() {
            for (slot, id) in vector.iter_mut().zip([
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ]) {
                if let Some(id) = id {
                    *slot = Some(
                        pool.handle_for(id)
                            .ok_or_else(|| GpuPickError::MissingColumn(id.clone()))?,
                    );
                }
            }
        }
        self.prepare_series_buffer(
            pool.buffer(),
            descriptor,
            GpuStreamPickColumns {
                x: x_handle,
                y: y_handle,
                style_index: style_index_handle,
                size_value: size_value_handle,
                vector,
            },
            Some(pool),
        )
    }

    fn prepare_series_buffer(
        &self,
        buffer: &wgpu::Buffer,
        descriptor: GpuPickSeriesDescriptor<'_>,
        columns: GpuStreamPickColumns,
        resident: Option<&ColumnPool>,
    ) -> Result<PickSeriesSlot, GpuPickError> {
        let limits = self.bundle.device.limits();
        if buffer.size() > limits.max_storage_buffer_binding_size {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: buffer.size(),
                limit: limits.max_storage_buffer_binding_size,
            });
        }
        let x_handle = columns.x;
        let y_handle = columns.y;
        for handle in [
            Some(x_handle),
            Some(y_handle),
            columns.style_index,
            columns.size_value,
            columns.vector[0],
            columns.vector[1],
            columns.vector[2],
        ]
        .into_iter()
        .flatten()
        {
            validate_stream_handle(buffer, handle)?;
        }
        let x_allocation_epoch = resident
            .and_then(|pool| pool.allocation_epoch(&descriptor.x_column))
            .unwrap_or(0);
        let y_allocation_epoch = resident
            .and_then(|pool| pool.allocation_epoch(&descriptor.y_column))
            .unwrap_or(0);
        let paired_len = x_handle.len_values.min(y_handle.len_values);
        if paired_len == 0 {
            return Err(GpuPickError::EmptySeries(descriptor.series_id));
        }
        let point_count = u32::try_from(paired_len).map_err(|_| GpuPickError::TooManyValues {
            series_id: descriptor.series_id.clone(),
            count: paired_len,
        })?;

        let mut flags = 0u32;
        let mut base_radius_px = 0.0;
        let mut base_shape_id = 0u32;
        let mut max_extent_px = 0.0;
        let mut style_count = 0u32;
        let mut override_count = 0u32;
        let mut style_index_column = None;
        let mut style_index_handle = None;
        let mut style_index_allocation_epoch = None;
        let mut style_index_base = 0u32;
        let mut style_index_len = 0u32;
        let mut size_column = None;
        let mut size_handle = None;
        let mut size_allocation_epoch = None;
        let mut size_base = 0u32;
        let mut size_len = 0u32;
        let mut size_domain = [0.0f32; 4];
        let mut size_px = [0.0f32; 2];

        let dummy_slot = [ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [0.0; 4],
        }];
        let dummy_override = [ScatterStyleOverrideGpu {
            point_index: 0,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [0.0; 4],
        }];
        let mut slot_bytes: &[u8] = bytemuck::cast_slice(&dummy_slot);
        let mut override_bytes: &[u8] = bytemuck::cast_slice(&dummy_override);

        if let Some(scatter) = descriptor.scatter.as_ref() {
            flags |= FLAG_SCATTER;
            base_radius_px = nonnegative(scatter.base_radius_px);
            base_shape_id = scatter.base_shape_id;
            max_extent_px = style_max_extent(&descriptor.series_id, scatter)?;
            if let Some(map) = scatter.style_map.as_ref() {
                flags |= FLAG_STYLE_MAP;
                style_count = map.style_meta.style_count;
                override_count = map.style_meta.override_count;
                if !map.style_slots.is_empty() {
                    slot_bytes = bytemuck::cast_slice(map.style_slots);
                }
                if !map.style_overrides.is_empty() {
                    override_bytes = bytemuck::cast_slice(map.style_overrides);
                }
                if let Some(column_id) = map.style_index_column.as_ref() {
                    flags |= FLAG_STYLE_INDEX;
                    let handle = columns
                        .style_index
                        .ok_or_else(|| GpuPickError::MissingColumn(column_id.clone()))?;
                    let allocation_epoch = resident
                        .and_then(|pool| pool.allocation_epoch(column_id))
                        .unwrap_or(0);
                    style_index_base = lane_base(handle, &descriptor.series_id)?;
                    style_index_len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
                    style_index_column = Some(column_id.clone());
                    style_index_handle = Some(handle);
                    style_index_allocation_epoch = Some(allocation_epoch);
                }
            }
            if let Some(size_map) = scatter.size_map.as_ref() {
                flags |= FLAG_SIZE_MAP;
                if size_map.area {
                    flags |= FLAG_SIZE_AREA;
                }
                let handle = columns
                    .size_value
                    .ok_or_else(|| GpuPickError::MissingColumn(size_map.column.clone()))?;
                let allocation_epoch = resident
                    .and_then(|pool| pool.allocation_epoch(&size_map.column))
                    .unwrap_or(0);
                size_base = lane_base(handle, &descriptor.series_id)?;
                size_len = u32::try_from(handle.len_values).unwrap_or(u32::MAX);
                size_column = Some(size_map.column.clone());
                size_handle = Some(handle);
                size_allocation_epoch = Some(allocation_epoch);
                let (min_hi, min_lo) = crate::data::split_f64_to_f32_pair(size_map.domain_min);
                let (max_hi, max_lo) = crate::data::split_f64_to_f32_pair(size_map.domain_max);
                size_domain = [min_hi, min_lo, max_hi, max_lo];
                size_px = [nonnegative(size_map.min_px), nonnegative(size_map.max_px)];
            }
        }

        // A point vector's arrows: every component column is resident and
        // the arrow count is the shortest column the vector shader reads.
        let mut vector_handles = [None; 3];
        let mut vector_epochs = [None; 3];
        let mut vector_base = [0u32; 2];
        let mut vector_count = 0u32;
        if let Some(arrows) = descriptor.vector.as_ref() {
            if descriptor.scatter.is_none() {
                return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
            }
            flags |= FLAG_VECTOR;
            let ids = [
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ];
            let mut count = paired_len;
            for (index, id) in ids.into_iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let handle =
                    columns.vector[index].ok_or_else(|| GpuPickError::MissingColumn(id.clone()))?;
                count = count.min(handle.len_values);
                vector_handles[index] = Some(handle);
                vector_epochs[index] = Some(
                    resident
                        .and_then(|pool| pool.allocation_epoch(id))
                        .unwrap_or(0),
                );
                if index < 2 {
                    vector_base[index] = lane_base(handle, &descriptor.series_id)?;
                }
            }
            vector_count = u32::try_from(count).unwrap_or(u32::MAX);
        }

        let line_half_width_px = descriptor
            .line_width_px
            .map(|width| nonnegative(width) * 0.5)
            .unwrap_or(0.0);
        if descriptor.line_width_px.is_some() {
            flags |= FLAG_LINE;
            max_extent_px = max_extent_px.max(line_half_width_px);
        }
        let line_step_mode = crate::line_step::step_mode(descriptor.line_interpolation);
        let mut line_dash = [0.0f32; 8];
        let pattern = descriptor.line_style.pattern();
        for (slot, length) in line_dash.iter_mut().zip(pattern) {
            *slot = *length;
        }
        let line_dash_len = pattern.len().min(line_dash.len()) as u32;
        if flags & (FLAG_SCATTER | FLAG_LINE) == 0 {
            return Err(GpuPickError::NoPickPrimitive(descriptor.series_id));
        }

        let _ = lane_base(x_handle, &descriptor.series_id)?;
        let _ = lane_base(y_handle, &descriptor.series_id)?;
        let (gate_word_count, workgroup_count, dispatch_x, dispatch_y) =
            gate_dispatch_layout(point_count, limits.max_compute_workgroups_per_dimension)?;
        // An arrow reaches as far from its anchor as its component says, so
        // no per-axis gate bounds it: a vector series scans every point.
        let direct_scan = point_count <= GPU_PICK_DIRECT_SCAN_POINTS || flags & FLAG_VECTOR != 0;
        let allocated_gate_words = if direct_scan { 1 } else { gate_word_count };
        let gate_mask_bytes = u64::from(allocated_gate_words)
            .checked_mul(GATE_MASK_BYTES)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "GPU pick gate-mask buffer",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;
        let workgroup_candidate_bytes = u64::from(workgroup_count)
            .checked_mul(CANDIDATE_BYTES)
            .ok_or(GpuPickError::DeviceLimit {
                resource: "GPU pick workgroup-candidate buffer",
                requested: u64::MAX,
                limit: limits.max_buffer_size,
            })?;
        let storage_limit = limits.max_storage_buffer_binding_size;
        if gate_mask_bytes > limits.max_buffer_size || gate_mask_bytes > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "GPU pick gate-mask buffer",
                requested: gate_mask_bytes,
                limit: limits.max_buffer_size.min(storage_limit),
            });
        }
        if workgroup_candidate_bytes > limits.max_buffer_size
            || workgroup_candidate_bytes > storage_limit
        {
            return Err(GpuPickError::DeviceLimit {
                resource: "GPU pick workgroup-candidate buffer",
                requested: workgroup_candidate_bytes,
                limit: limits.max_buffer_size.min(storage_limit),
            });
        }

        // Every buffer this slot owns is tallied by the creation helpers, so the
        // charge below cannot disagree with what was allocated.
        let series_tally = ChargeTally::new();
        let gate_masks = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick XY gate masks"),
                size: gate_mask_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "XY gate-mask buffer",
        )?;
        let workgroup_candidates = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick exact workgroup candidates"),
                size: workgroup_candidate_bytes,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            },
            "exact workgroup-candidate buffer",
        )?;
        let query_params = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick query params"),
                size: std::mem::size_of::<PickQueryParamsGpu>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "query uniform",
        )?;
        let style_slots = create_buffer_init_checked(
            &series_tally,
            &self.bundle.device,
            "figgy GPU pick style slots",
            slot_bytes,
            wgpu::BufferUsages::STORAGE,
            "style slot buffer",
        )?;
        let style_overrides = create_buffer_init_checked(
            &series_tally,
            &self.bundle.device,
            "figgy GPU pick style overrides",
            override_bytes,
            wgpu::BufferUsages::STORAGE,
            "style override buffer",
        )?;
        let result = create_buffer_checked(
            &series_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick series scalar"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            },
            "series pick scalar",
        )?;

        let query_data_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick query data bg"),
                layout: &self.bundle.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: gate_masks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: workgroup_candidates.as_entire_binding(),
                    },
                ],
            },
            "query data bind group",
        )?;
        let query_work_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick query work bg"),
                layout: &self.bundle.query_work_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: query_params.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: style_slots.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: style_overrides.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: result.as_entire_binding(),
                    },
                ],
            },
            "query work bind group",
        )?;
        let reduce_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick per-series reduction bg"),
                layout: &self.bundle.reduce_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: workgroup_candidates.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: result.as_entire_binding(),
                    },
                ],
            },
            "per-series reduction bind group",
        )?;

        let identity = PickIdentity {
            source_id: descriptor.source_id,
            series_id: descriptor.series_id,
        };
        // One charge for the whole slot: the style rows and params live inside
        // the bind groups above, so per-buffer tracking cannot see them. The
        // tally already holds the sizes the device was handed, so there is no
        // second list of terms to keep in step with the creations.
        Ok(PickSeriesSlot {
            identity,
            gpu: PickSeriesGpu {
                _charge: shared_charge(
                    series_tally,
                    &self.bundle.ledger,
                    GpuResourceKind::PickScratch,
                ),
                pool_identity: resident.map(ColumnPool::identity),
                x_column: descriptor.x_column,
                y_column: descriptor.y_column,
                style_index_column,
                x_handle,
                y_handle,
                style_index_handle,
                pool_layout_generation: resident.map_or(0, ColumnPool::layout_generation),
                x_allocation_epoch,
                y_allocation_epoch,
                style_index_allocation_epoch,
                size_column,
                size_handle,
                size_allocation_epoch,
                vector: descriptor.vector,
                vector_handles,
                vector_epochs,
                vector_base,
                vector_count,
                size_base,
                size_len,
                size_domain,
                size_px,
                query_params,
                gate_masks,
                workgroup_candidates,
                query_data_bg,
                query_work_bg,
                reduce_bg,
                result,
                point_count,
                gate_word_count,
                dispatch_x,
                dispatch_y,
                direct_scan,
                flags,
                base_radius_px,
                base_shape_id,
                line_half_width_px,
                line_step_mode,
                line_dash,
                line_dash_len,
                max_extent_px,
                style_count,
                override_count,
                style_index_base,
                style_index_len,
            },
        })
    }

    #[cfg(test)]
    fn current_reuse_slots<'a>(&self, skip: Option<usize>) -> Vec<GpuPickRegistrySlot<'a>> {
        self.registry
            .slots
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != skip)
            .map(|(current_index, slot)| GpuPickRegistrySlot::Reuse {
                current_index,
                source_id: slot.identity.source_id.clone(),
                series_id: slot.identity.series_id.clone(),
            })
            .collect()
    }

    fn check_pool_identity(&self, pool: &ColumnPool) -> Result<PoolIdentity, GpuPickError> {
        let target = pool.identity();
        if let Some(current) = self.registry.pool_identity.as_ref()
            && !current.same_instance(&target)
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        Ok(target)
    }

    fn prepare_reused_gpu(
        &self,
        pool: &ColumnPool,
        target_pool_identity: &PoolIdentity,
        current: &PickSeriesSlot,
        next_series_id: &str,
    ) -> Result<PickSeriesGpu, GpuPickError> {
        let series = &current.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(target_pool_identity))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }

        let relocated = |id: &ColumnId,
                         expected: ColumnHandle,
                         expected_epoch: u64|
         -> Result<ColumnHandle, GpuPickError> {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch)
                || current.len_values != expected.len_values
                || current.byte_size != expected.byte_size
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: next_series_id.to_owned(),
                    column_id: id.clone(),
                });
            }
            Ok(current)
        };

        let x_handle = relocated(&series.x_column, series.x_handle, series.x_allocation_epoch)?;
        let y_handle = relocated(&series.y_column, series.y_handle, series.y_allocation_epoch)?;
        let _ = lane_base(x_handle, next_series_id)?;
        let _ = lane_base(y_handle, next_series_id)?;
        let (style_index_handle, style_index_base, style_index_len) =
            if let Some(id) = series.style_index_column.as_ref() {
                let expected = series
                    .style_index_handle
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch = series
                    .style_index_allocation_epoch
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                let base = lane_base(current, next_series_id)?;
                let len =
                    u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                        series_id: next_series_id.to_owned(),
                        count: current.len_values,
                    })?;
                (Some(current), base, len)
            } else {
                (None, 0, 0)
            };

        let (size_handle, size_base, size_len) = if let Some(id) = series.size_column.as_ref() {
            let expected = series.size_handle.ok_or(GpuPickError::InvalidGpuResult)?;
            let expected_epoch = series
                .size_allocation_epoch
                .ok_or(GpuPickError::InvalidGpuResult)?;
            let current = relocated(id, expected, expected_epoch)?;
            let base = lane_base(current, next_series_id)?;
            let len =
                u32::try_from(current.len_values).map_err(|_| GpuPickError::TooManyValues {
                    series_id: next_series_id.to_owned(),
                    count: current.len_values,
                })?;
            (Some(current), base, len)
        } else {
            (None, 0, 0)
        };

        let mut vector_handles = [None; 3];
        let mut vector_base = [0u32; 2];
        if let Some(arrows) = series.vector.as_ref() {
            let ids = [
                Some(&arrows.u_column),
                Some(&arrows.v_column),
                arrows.color_column.as_ref(),
            ];
            for (index, id) in ids.into_iter().enumerate() {
                let Some(id) = id else {
                    continue;
                };
                let expected =
                    series.vector_handles[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let expected_epoch =
                    series.vector_epochs[index].ok_or(GpuPickError::InvalidGpuResult)?;
                let current = relocated(id, expected, expected_epoch)?;
                if index < 2 {
                    vector_base[index] = lane_base(current, next_series_id)?;
                }
                vector_handles[index] = Some(current);
            }
        }

        let storage_unchanged = pool.layout_generation() == series.pool_layout_generation
            && same_storage(x_handle, series.x_handle)
            && same_storage(y_handle, series.y_handle)
            && style_index_handle
                .zip(series.style_index_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && style_index_handle.is_some() == series.style_index_handle.is_some()
            && size_handle
                .zip(series.size_handle)
                .is_none_or(|(current, expected)| same_storage(current, expected))
            && size_handle.is_some() == series.size_handle.is_some()
            && vector_handles
                .iter()
                .zip(&series.vector_handles)
                .all(|(current, expected)| match (current, expected) {
                    (Some(current), Some(expected)) => same_storage(*current, *expected),
                    (None, None) => true,
                    _ => false,
                });
        if storage_unchanged {
            return Ok(series.clone());
        }

        let query_data_bg = create_bind_group_checked(
            &self.bundle.device,
            &wgpu::BindGroupDescriptor {
                label: Some("figgy GPU pick rebound query data bg"),
                layout: &self.bundle.query_data_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: pool.buffer().as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: series.gate_masks.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: series.workgroup_candidates.as_entire_binding(),
                    },
                ],
            },
            "rebound query data bind group",
        )?;

        let mut rebound = series.clone();
        rebound.pool_identity = Some(target_pool_identity.clone());
        rebound.x_handle = x_handle;
        rebound.y_handle = y_handle;
        rebound.style_index_handle = style_index_handle;
        rebound.pool_layout_generation = pool.layout_generation();
        rebound.style_index_base = style_index_base;
        rebound.style_index_len = style_index_len;
        rebound.size_handle = size_handle;
        rebound.size_base = size_base;
        rebound.size_len = size_len;
        rebound.vector_handles = vector_handles;
        rebound.vector_base = vector_base;
        rebound.query_data_bg = query_data_bg;
        Ok(rebound)
    }

    fn prepare_next_registry<'a>(
        &self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PickRegistry, GpuPickError> {
        #[cfg(test)]
        if self.fail_next_registry_prepare.replace(false) {
            return Err(GpuPickError::AllocationFailed {
                resource: "injected registry prepare failure",
            });
        }
        let storage_limit = self.bundle.device.limits().max_storage_buffer_binding_size;
        if pool.capacity() > storage_limit {
            return Err(GpuPickError::DeviceLimit {
                resource: "column-pool storage binding",
                requested: pool.capacity(),
                limit: storage_limit,
            });
        }
        let target_pool_identity = self.check_pool_identity(pool)?;

        let iterator = final_slots.into_iter();
        let mut plans = Vec::new();
        plans
            .try_reserve(iterator.size_hint().0)
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition plan",
            })?;
        plans.extend(iterator);

        let mut seen = Vec::new();
        seen.try_reserve_exact(self.registry.slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry transition index map",
            })?;
        seen.resize(self.registry.slots.len(), false);

        let mut slots = Vec::new();
        slots
            .try_reserve_exact(plans.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry successor slots",
            })?;
        for plan in plans {
            let slot = match plan {
                GpuPickRegistrySlot::Reuse {
                    current_index,
                    source_id,
                    series_id,
                } => {
                    let len = self.registry.slots.len();
                    let Some(current) = self.registry.slots.get(current_index) else {
                        return Err(GpuPickError::InvalidSeriesIndex {
                            index: current_index,
                            len,
                        });
                    };
                    if std::mem::replace(&mut seen[current_index], true) {
                        return Err(GpuPickError::DuplicateSeriesIndex {
                            index: current_index,
                        });
                    }
                    PickSeriesSlot {
                        gpu: self.prepare_reused_gpu(
                            pool,
                            &target_pool_identity,
                            current,
                            &series_id,
                        )?,
                        identity: PickIdentity {
                            source_id,
                            series_id,
                        },
                    }
                }
                GpuPickRegistrySlot::Build(descriptor) => self.prepare_series(pool, descriptor)?,
            };
            slots.push(slot);
        }

        let mut identities = Vec::new();
        identities
            .try_reserve_exact(slots.len())
            .map_err(|_| GpuPickError::AllocationFailed {
                resource: "registry identity snapshot",
            })?;
        identities.extend(slots.iter().map(|slot| slot.identity.clone()));
        let reduce = self.prepare_reduce_resources(slots.len())?;
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;

        Ok(PickRegistry {
            slots,
            identities: identities.into(),
            reduce,
            pool_identity: Some(target_pool_identity),
            pool_layout_generation: pool.layout_generation(),
            generation,
        })
    }

    pub(crate) fn prepare_registry_transition<'engine, 'a>(
        &'engine mut self,
        pool: &ColumnPool,
        final_slots: impl IntoIterator<Item = GpuPickRegistrySlot<'a>>,
    ) -> Result<PreparedPickRegistryTransition<'engine>, GpuPickError> {
        let next = self.prepare_next_registry(pool, final_slots)?;
        Ok(PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        })
    }

    /// Replace one registry slot without recreating any other series resources.
    ///
    /// The replacement is built first, so a build/allocation error leaves the
    /// existing slot untouched. The returned id is the unchanged positional
    /// slot; later-series equal-distance precedence is therefore preserved.
    #[cfg(test)]
    fn replace_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(Some(index));
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Insert one registry slot without recreating existing series resources.
    ///
    /// The new series is built completely at the tail before it is moved into
    /// position, so a build/allocation error leaves the registry and its tie
    /// order untouched. An index equal to the registry length appends it.
    #[cfg(test)]
    fn insert_series_at(
        &mut self,
        index: usize,
        pool: &ColumnPool,
        descriptor: GpuPickSeriesDescriptor<'_>,
    ) -> Result<GpuPickSeriesId, GpuPickError> {
        let len = self.registry.slots.len();
        if index > len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let mut final_slots = self.current_reuse_slots(None);
        final_slots.insert(index, GpuPickRegistrySlot::Build(descriptor));
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(GpuPickSeriesId(index as u32))
    }

    /// Remove one positional registry slot without recreating surviving slots.
    /// Later slots shift left, matching their new production traversal order.
    #[cfg(test)]
    fn remove_series_at(&mut self, index: usize) -> Result<(), GpuPickError> {
        let len = self.registry.slots.len();
        if index >= len {
            return Err(GpuPickError::InvalidSeriesIndex { index, len });
        }
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .ok_or(GpuPickError::RegistryGenerationExhausted)?;
        let mut slots = self.registry.slots.clone();
        slots.remove(index);
        let identities = slots
            .iter()
            .map(|slot| slot.identity.clone())
            .collect::<Vec<_>>()
            .into();
        let next = PickRegistry {
            slots,
            identities,
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        }
        .commit();
        Ok(())
    }

    /// Drop every registered series while retaining reusable pipelines and
    /// reduction storage.
    #[cfg(test)]
    fn clear_series(&mut self) {
        let generation = self
            .registry
            .generation
            .checked_add(1)
            .expect("GPU pick registry generation exhausted while clearing");
        let next = PickRegistry {
            slots: Vec::new(),
            identities: Arc::from([]),
            reduce: self.registry.reduce.clone(),
            pool_identity: self.registry.pool_identity.clone(),
            pool_layout_generation: self.registry.pool_layout_generation,
            generation,
        };
        PreparedPickRegistryTransition {
            engine: self,
            next: Some(next),
        }
        .commit();
    }

    /// Rebind registered columns after an in-place [`ColumnPool::defragment`].
    ///
    /// Defragmentation changes the backing buffer, offsets, and generations,
    /// but preserves every value. This method verifies every referenced column
    /// still exists with exactly its registered logical length, recreates the
    /// whole-pool data bind groups, and updates all handles/bases without
    /// reallocating gate or reduction storage.
    ///
    /// Use this only with the same `ColumnPool` after a relocation-only
    /// defragment. It is not valid after replacing column contents, even when
    /// ids and lengths happen to match; call replace_series_at/rebuild instead.
    #[cfg(test)]
    fn rebind_columns(&mut self, pool: &ColumnPool) -> Result<(), GpuPickError> {
        let final_slots = self.current_reuse_slots(None);
        self.prepare_registry_transition(pool, final_slots)?
            .commit();
        Ok(())
    }

    fn validate_series_columns(
        pool: &ColumnPool,
        slot: &PickSeriesSlot,
    ) -> Result<(), GpuPickError> {
        let series = &slot.gpu;
        if !series
            .pool_identity
            .as_ref()
            .is_some_and(|identity| identity.same_instance(&pool.identity()))
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if pool.layout_generation() != series.pool_layout_generation {
            return Err(GpuPickError::StaleColumn {
                series_id: slot.identity.series_id.clone(),
                column_id: series.x_column.clone(),
            });
        }

        for (id, expected, expected_epoch) in [
            (&series.x_column, series.x_handle, series.x_allocation_epoch),
            (&series.y_column, series.y_handle, series.y_allocation_epoch),
        ] {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        for (id, expected, expected_epoch) in [
            (
                series.style_index_column.as_ref(),
                series.style_index_handle,
                series.style_index_allocation_epoch,
            ),
            (
                series.size_column.as_ref(),
                series.size_handle,
                series.size_allocation_epoch,
            ),
            (
                series.vector.as_ref().map(|vector| &vector.u_column),
                series.vector_handles[0],
                series.vector_epochs[0],
            ),
            (
                series.vector.as_ref().map(|vector| &vector.v_column),
                series.vector_handles[1],
                series.vector_epochs[1],
            ),
            (
                series
                    .vector
                    .as_ref()
                    .and_then(|vector| vector.color_column.as_ref()),
                series.vector_handles[2],
                series.vector_epochs[2],
            ),
        ]
        .into_iter()
        .filter_map(|(id, expected, epoch)| Some((id?, expected?, epoch?)))
        {
            let Some(current) = pool.handle_for(id) else {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            };
            if pool.allocation_epoch(id) != Some(expected_epoch) || !same_storage(current, expected)
            {
                return Err(GpuPickError::StaleColumn {
                    series_id: slot.identity.series_id.clone(),
                    column_id: id.clone(),
                });
            }
        }
        Ok(())
    }

    /// Submit one exact pick query and return an owned async readback ticket.
    #[cfg(test)]
    fn pick(&self, pool: &ColumnPool, query: GpuPickQuery) -> Result<GpuPickTicket, GpuPickError> {
        self.pick_with_display_scale(pool, query, 1.0)
    }

    /// Submit one exact pick query using the display's uniform pixel scale.
    ///
    /// Series descriptors remain in logical-document pixels. This scale is
    /// applied once by the query shader to scatter radii, line half-widths,
    /// and the matching conservative gate widths. Cursor coordinates,
    /// `max_distance_px`, and returned distances remain physical canvas pixels.
    /// The registered series, in the order a pick addresses them.
    pub(crate) fn series_identities(&self) -> GpuPickSeriesIdentities {
        GpuPickSeriesIdentities(Arc::clone(&self.registry.identities))
    }

    #[cfg(test)]
    pub(crate) fn pick_with_display_scale(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let mut exact = PickExactPipelines::default();
        for key in self.exact_variants(&query) {
            exact.ensure(&self.bundle, key)?;
        }
        self.pick_with_arcs(pool, query, display_scale, &[], &exact)
    }

    /// The exact module and variant each registered series needs under
    /// `query`, in registry order: create them
    /// ([`PickExactPipelines::ensure`]) before [`Self::pick_with_arcs`].
    pub(crate) fn exact_variants<'a>(
        &'a self,
        query: &'a GpuPickQuery,
    ) -> impl Iterator<Item = PickExactKey> + 'a {
        self.registry
            .slots
            .iter()
            .map(move |slot| series_exact_variant(&slot.gpu, query))
    }

    /// The bundle the exact pipelines are created against.
    pub(crate) fn bundle(&self) -> &PickPipelineBundle {
        &self.bundle
    }

    /// [`Self::pick_with_display_scale`] with each registered series' arc
    /// prefix, by registry position: the per-point cumulative arc length the
    /// line shader drew the series with (`line_arc.wgsl`) for the query's
    /// transform, which places its dash phase and sketch wobble. A missing
    /// entry, or a buffer too short for the series, picks without it.
    /// `exact` must hold every variant [`Self::exact_variants`] names.
    pub(crate) fn pick_with_arcs(
        &self,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
        arcs: &[Option<Arc<wgpu::Buffer>>],
        exact: &PickExactPipelines,
    ) -> Result<GpuPickTicket, GpuPickError> {
        let [cursor_x, cursor_y] = query.canvas_position_px;
        if !display_scale.is_finite()
            || display_scale <= 0.0
            || !cursor_x.is_finite()
            || !cursor_y.is_finite()
            || !query.max_distance_px.is_finite()
            || query.max_distance_px < 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        let [chart_x, chart_y, chart_width, chart_height] = query.chart_rect_px;
        if !chart_x.is_finite()
            || !chart_y.is_finite()
            || !chart_width.is_finite()
            || !chart_height.is_finite()
            || chart_width <= 0.0
            || chart_height <= 0.0
        {
            return Ok(GpuPickTicket::ready_none());
        }
        if let Some([x, y, width, height]) = query.data_area_px {
            let x1 = x + width;
            let y1 = y + height;
            if !x.is_finite()
                || !y.is_finite()
                || !width.is_finite()
                || !height.is_finite()
                || cursor_x < x
                || cursor_x > x1
                || cursor_y < y
                || cursor_y > y1
            {
                return Ok(GpuPickTicket::ready_none());
            }
        }
        if let Some(expected) = self.registry.pool_identity.as_ref()
            && !expected.same_instance(&pool.identity())
        {
            return Err(GpuPickError::ForeignColumnPool);
        }
        if self.registry.slots.is_empty() {
            return Ok(GpuPickTicket::ready_none());
        }
        if self.registry.pool_layout_generation != pool.layout_generation() {
            let first = &self.registry.slots[0];
            return Err(GpuPickError::StaleColumn {
                series_id: first.identity.series_id.clone(),
                column_id: first.gpu.x_column.clone(),
            });
        }

        for (series_order, slot) in self.registry.slots.iter().enumerate() {
            Self::validate_series_columns(pool, slot)?;
            let series = &slot.gpu;
            let params = query_params(
                series,
                &slot.identity.series_id,
                &query,
                display_scale,
                series_order as u32,
                [
                    series.point_count,
                    lane_base(series.x_handle, &slot.identity.series_id)?,
                    lane_base(series.y_handle, &slot.identity.series_id)?,
                    series.style_index_base,
                ],
                [
                    0,
                    series.point_count,
                    series.point_count.saturating_sub(1),
                    0,
                ],
                series_arc(arcs, series_order, series.point_count).is_some(),
            );
            self.bundle
                .queue
                .write_buffer(&series.query_params, 0, bytemuck::bytes_of(&params));
        }

        let readback_tally = ChargeTally::new();
        let readback = create_buffer_checked(
            &readback_tally,
            &self.bundle.device,
            &wgpu::BufferDescriptor {
                label: Some("figgy GPU pick detached readback"),
                size: CANDIDATE_BYTES,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            },
            "detached pick readback",
        )?;
        let mut encoder =
            self.bundle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("figgy exact GPU pick encoder"),
                });
        // Reduction storage may intentionally be larger than the live registry
        // after replace/remove. Clear every slot so a prior query's candidate
        // can never participate as a stale tail entry.
        encoder.clear_buffer(&self.registry.reduce.candidates, 0, None);
        let has_gated_series = self.registry.slots.iter().any(|slot| !slot.gpu.direct_scan);
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick X gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_x);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        if has_gated_series {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick Y gate"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.gate_y);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                if series.direct_scan {
                    continue;
                }
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick candidate scan"),
                timestamp_writes: None,
            });
            for (series_order, slot) in self.registry.slots.iter().enumerate() {
                let series = &slot.gpu;
                let key = series_exact_variant(series, &query);
                pass.set_pipeline(exact.get(key).ok_or(GpuPickError::InvalidGpuResult)?);
                pass.set_bind_group(0, &series.query_data_bg, &[]);
                pass.set_bind_group(1, &series.query_work_bg, &[]);
                // The pass keeps the bind group alive until it is submitted.
                let arc = series_arc(arcs, series_order, series.point_count)
                    .map(|buffer| self.bundle.arc_bind_group(buffer));
                pass.set_bind_group(2, arc.as_ref().unwrap_or(&self.bundle.arc_placeholder), &[]);
                pass.dispatch_workgroups(series.dispatch_x, series.dispatch_y, 1);
            }
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy exact GPU pick per-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            for slot in &self.registry.slots {
                let series = &slot.gpu;
                pass.set_bind_group(0, &series.reduce_bg, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
        }
        for (index, slot) in self.registry.slots.iter().enumerate() {
            let series = &slot.gpu;
            encoder.copy_buffer_to_buffer(
                &series.result,
                0,
                &self.registry.reduce.candidates,
                index as u64 * CANDIDATE_BYTES,
                CANDIDATE_BYTES,
            );
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("figgy GPU pick cross-series reduction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.bundle.reduce);
            pass.set_bind_group(0, &self.registry.reduce.bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.registry.reduce.final_result,
            0,
            &readback,
            0,
            CANDIDATE_BYTES,
        );
        self.bundle.queue.submit(std::iter::once(encoder.finish()));

        let slice = readback.slice(..CANDIDATE_BYTES);
        let (sender, receiver) = oneshot::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        Ok(GpuPickTicket {
            state: GpuPickTicketState::Pending {
                device: Arc::clone(&self.bundle.device),
                readback,
                receiver,
                identities: Arc::clone(&self.registry.identities),
                _charge: readback_tally.into_charge(&self.bundle.ledger, GpuResourceKind::Readback),
                _scratch_charge: None,
            },
        })
    }
}

/// The arc prefix of the series at registry position `index`, when one was
/// supplied and holds a value for each of its `point_count` points.
fn series_arc(
    arcs: &[Option<Arc<wgpu::Buffer>>],
    index: usize,
    point_count: u32,
) -> Option<&wgpu::Buffer> {
    arcs.get(index)
        .and_then(Option::as_deref)
        .filter(|buffer| buffer.size() >= u64::from(point_count) * 4)
}

/// The identities of a registry's series in registry order.
pub(crate) struct GpuPickSeriesIdentities(Arc<[PickIdentity]>);

impl GpuPickSeriesIdentities {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn series_ids(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|identity| identity.series_id.as_str())
    }
}

enum GpuPickTicketState {
    ReadyNone,
    Pending {
        device: Arc<wgpu::Device>,
        readback: wgpu::Buffer,
        receiver: oneshot::Receiver<Result<(), wgpu::BufferAsyncError>>,
        identities: Arc<[PickIdentity]>,
        /// Credited when the ticket resolves or is abandoned — either way the
        /// MAP_READ buffer goes with it.
        _charge: GpuByteCharge,
        _scratch_charge: Option<SharedCharge>,
    },
}

/// Owned, detached asynchronous scalar readback.
pub struct GpuPickTicket {
    state: GpuPickTicketState,
}

pub(crate) struct GpuIndexedPick {
    pub(crate) source_id: Option<String>,
    pub(crate) series_id: String,
    pub(crate) series_order: u32,
    pub(crate) point_index: u32,
    pub(crate) distance_px: f32,
}

impl GpuPickTicket {
    fn ready_none() -> Self {
        Self {
            state: GpuPickTicketState::ReadyNone,
        }
    }

    pub async fn resolve(self) -> Result<Option<PickedPoint>, GpuPickError> {
        Ok(self.resolve_indexed().await?.map(|picked| PickedPoint {
            source_id: picked.source_id,
            series_id: picked.series_id,
            point_index: picked.point_index as usize,
            distance_px: picked.distance_px,
        }))
    }

    pub(crate) async fn resolve_indexed(self) -> Result<Option<GpuIndexedPick>, GpuPickError> {
        let GpuPickTicketState::Pending {
            device: _device,
            readback,
            receiver,
            identities,
            // Dropped here: resolving consumes the readback buffer.
            _charge,
            _scratch_charge,
        } = self.state
        else {
            return Ok(None);
        };

        #[cfg(not(target_arch = "wasm32"))]
        let _ = _device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .await
            .map_err(|_| GpuPickError::MapChannelClosed)?
            .map_err(GpuPickError::MapFailed)?;
        let slice = readback.slice(..CANDIDATE_BYTES);
        let mapped = slice
            .get_mapped_range()
            .expect("GPU pick readback is mapped after map_async");
        let candidate = *bytemuck::from_bytes::<PickCandidateGpu>(&mapped);
        drop(mapped);
        readback.unmap();

        match candidate.valid {
            0 => Ok(None),
            2 => Err(GpuPickError::TraversalOverflow),
            1 => {
                let identity = identities
                    .get(candidate.series_order as usize)
                    .ok_or(GpuPickError::InvalidGpuResult)?;
                if !candidate.distance_px.is_finite() {
                    return Err(GpuPickError::InvalidGpuResult);
                }
                Ok(Some(GpuIndexedPick {
                    source_id: identity.source_id.clone(),
                    series_id: identity.series_id.clone(),
                    series_order: candidate.series_order,
                    point_index: candidate.point_index,
                    distance_px: candidate.distance_px,
                }))
            }
            _ => Err(GpuPickError::InvalidGpuResult),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::InitPhase;
    use crate::color::Color;
    use crate::config::{AxisScale, Config};
    use crate::data::{Column, split_f64_to_f32_pair};
    use crate::data_config::{
        DataLineStyleConfig, DataRenderType, DataScatterPointStyleConfig,
        DataScatterPointStyleOverride, DataScatterStyleConfig, ScatterShape, SeriesConfig,
    };
    use crate::layout::{ChartArea, Rect};
    use crate::line::LineStylePreset;
    use crate::pick::{PointColumnLookup, PointPickOptions, pick_nearest_point};

    /// The largest `visual_shape_radius` factor (the triangles').
    const MAX_SHAPE_SCALE: f32 = 1.555_120_3;
    /// `STROKE_HALF_PX`: an outline marker's stroke reaches this far either
    /// side of its contour.
    const STROKE_HALF: f32 = crate::pick::MARKER_STROKE_HALF_PX;

    struct CpuColumns(HashMap<ColumnId, Vec<f32>>);

    impl CpuColumns {
        fn new(entries: &[(&str, &[f32])]) -> Self {
            Self(
                entries
                    .iter()
                    .map(|(id, values)| ((*id).to_owned(), values.to_vec()))
                    .collect(),
            )
        }
    }

    impl PointColumnLookup for CpuColumns {
        fn get_f32_column(&self, id: &ColumnId) -> Option<&[f32]> {
            self.0.get(id).map(Vec::as_slice)
        }
    }

    fn f32_column(values: Vec<f32>) -> Column<f32> {
        let min = values.iter().copied().fold(f32::INFINITY, f32::min);
        let max = values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Column {
            data: values,
            min,
            max,
        }
    }

    fn f64_column(values: Vec<f64>) -> Column<f64> {
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Column {
            data: values,
            min,
            max,
        }
    }

    fn parity_config(width: u32, height: u32, x: (f64, f64), y: (f64, f64)) -> Config {
        let mut config = crate::default::default_config();
        config.chart_area = ChartArea(Rect {
            x: 0,
            y: 0,
            width,
            height,
        });
        config.bottom_x.min = x.0;
        config.bottom_x.max = x.1;
        config.left_y.min = y.0;
        config.left_y.max = y.1;
        config.chart_title.top_margin = 0.0;
        for axis in [
            &mut config.top_x,
            &mut config.bottom_x,
            &mut config.left_y,
            &mut config.right_y,
        ] {
            axis.out_margin = 0.0;
            axis.major_tick_length = 0.0;
        }
        config
    }

    fn query_from_config(
        config: &Config,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> GpuPickQuery {
        let chart = config.chart_area.0;
        let data_area_px = config.data_area().ok().map(|area| {
            let rect = area.0;
            [
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
            ]
        });
        GpuPickQuery {
            transform: crate::data_render::scatter_transform_from_config(config),
            chart_rect_px: [
                chart.x as f32,
                chart.y as f32,
                chart.width as f32,
                chart.height as f32,
            ],
            data_area_px,
            canvas_position_px,
            max_distance_px,
            draw_style: GpuPickDrawStyle::PRECISE,
        }
    }

    fn resolve_pick(
        engine: &GpuPickEngine,
        pool: &ColumnPool,
        query: GpuPickQuery,
    ) -> Option<PickedPoint> {
        pollster::block_on(engine.pick(pool, query).unwrap().resolve()).unwrap()
    }

    fn resolve_pick_scaled(
        engine: &GpuPickEngine,
        pool: &ColumnPool,
        query: GpuPickQuery,
        display_scale: f32,
    ) -> Option<PickedPoint> {
        pollster::block_on(
            engine
                .pick_with_display_scale(pool, query, display_scale)
                .unwrap()
                .resolve(),
        )
        .unwrap()
    }

    fn cpu_pick(
        config: &Config,
        series: &[SeriesConfig],
        columns: &CpuColumns,
        canvas_position_px: [f32; 2],
        max_distance_px: f32,
    ) -> Option<PickedPoint> {
        pick_nearest_point(
            config,
            series,
            columns,
            canvas_position_px[0],
            canvas_position_px[1],
            PointPickOptions { max_distance_px },
        )
    }

    fn assert_pick_parity(cpu: Option<PickedPoint>, gpu: Option<PickedPoint>) {
        match (cpu, gpu) {
            (None, None) => {}
            (Some(cpu), Some(gpu)) => {
                assert_eq!(gpu.source_id, cpu.source_id);
                assert_eq!(gpu.series_id, cpu.series_id);
                assert_eq!(gpu.point_index, cpu.point_index);
                assert!(
                    (gpu.distance_px - cpu.distance_px).abs() <= 0.001,
                    "GPU distance {} differs from CPU distance {}",
                    gpu.distance_px,
                    cpu.distance_px
                );
            }
            (cpu, gpu) => panic!("CPU/GPU picker mismatch: CPU={cpu:?}, GPU={gpu:?}"),
        }
    }

    fn scatter_series(
        series_id: &str,
        source_id: Option<&str>,
        x_column: &str,
        y_column: &str,
        scatter: DataScatterStyleConfig,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_owned(),
            source_id: source_id.map(str::to_owned),
            label: None,
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            render_type: DataRenderType::Scatter { scatter },
        }
    }

    fn line_series(
        series_id: &str,
        source_id: Option<&str>,
        x_column: &str,
        y_column: &str,
        line_width: f32,
    ) -> SeriesConfig {
        SeriesConfig {
            series_id: series_id.to_owned(),
            source_id: source_id.map(str::to_owned),
            label: None,
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            render_type: DataRenderType::Line {
                line: DataLineStyleConfig {
                    line_style: LineStylePreset::Solid,
                    line_color: Color::BLACK,
                    line_width,
                    fill: None,
                    fill_color: None,
                    interpolation: crate::data_config::LineInterpolation::Linear,
                },
            },
        }
    }

    fn scatter_descriptor(
        series_id: &str,
        x_column: &str,
        y_column: &str,
    ) -> GpuPickSeriesDescriptor<'static> {
        GpuPickSeriesDescriptor {
            source_id: Some(format!("source-{series_id}")),
            series_id: series_id.to_owned(),
            x_column: x_column.to_owned(),
            y_column: y_column.to_owned(),
            scatter: Some(GpuPickScatter {
                base_radius_px: 2.0,
                base_shape_id: 5,
                style_map: None,
                size_map: None,
            }),
            line_width_px: None,
            line_interpolation: crate::data_config::LineInterpolation::Linear,
            line_style: crate::line::LineStylePreset::Solid,
            vector: None,
        }
    }

    fn test_query(canvas_position_px: [f32; 2]) -> GpuPickQuery {
        GpuPickQuery {
            transform: ScatterTransform {
                data_min: [0.0, 0.0],
                data_max: [10.0, 10.0],
                data_min_lo: [0.0; 2],
                data_max_lo: [0.0; 2],
                scale_log: [0.0; 2],
                pixel_to_ndc: [0.02; 2],
                data_to_panel_scale: [1.0; 2],
                data_to_panel_offset: [0.0; 2],
                style_params: [[0.0; 4]; 3],
                polar_angle: [0.0; 4],
                polar_extent: [0.0; 4],
            },
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
            canvas_position_px,
            max_distance_px: 0.0,
            draw_style: GpuPickDrawStyle::PRECISE,
        }
    }

    fn handle_snapshot(handle: ColumnHandle) -> (u32, u64, u64, usize) {
        (
            handle.generation,
            handle.offset,
            handle.byte_size,
            handle.len_values,
        )
    }

    #[derive(Debug, PartialEq)]
    struct PickSeriesMetadataSnapshot {
        source_id: Option<String>,
        series_id: String,
        x_column: ColumnId,
        y_column: ColumnId,
        style_index_column: Option<ColumnId>,
        x_handle: (u32, u64, u64, usize),
        y_handle: (u32, u64, u64, usize),
        style_index_handle: Option<(u32, u64, u64, usize)>,
        pool_layout_generation: u64,
        x_allocation_epoch: u64,
        y_allocation_epoch: u64,
        style_index_allocation_epoch: Option<u64>,
        point_count: u32,
        gate_word_count: u32,
        dispatch_x: u32,
        dispatch_y: u32,
        direct_scan: bool,
        flags: u32,
        base_radius_bits: u32,
        base_shape_id: u32,
        line_half_width_bits: u32,
        max_extent_bits: u32,
        style_count: u32,
        override_count: u32,
        style_index_base: u32,
        style_index_len: u32,
    }

    impl PickSeriesMetadataSnapshot {
        fn capture(slot: &PickSeriesSlot) -> Self {
            let series = &slot.gpu;
            Self {
                source_id: slot.identity.source_id.clone(),
                series_id: slot.identity.series_id.clone(),
                x_column: series.x_column.clone(),
                y_column: series.y_column.clone(),
                style_index_column: series.style_index_column.clone(),
                x_handle: handle_snapshot(series.x_handle),
                y_handle: handle_snapshot(series.y_handle),
                style_index_handle: series.style_index_handle.map(handle_snapshot),
                pool_layout_generation: series.pool_layout_generation,
                x_allocation_epoch: series.x_allocation_epoch,
                y_allocation_epoch: series.y_allocation_epoch,
                style_index_allocation_epoch: series.style_index_allocation_epoch,
                point_count: series.point_count,
                gate_word_count: series.gate_word_count,
                dispatch_x: series.dispatch_x,
                dispatch_y: series.dispatch_y,
                direct_scan: series.direct_scan,
                flags: series.flags,
                base_radius_bits: series.base_radius_px.to_bits(),
                base_shape_id: series.base_shape_id,
                line_half_width_bits: series.line_half_width_px.to_bits(),
                max_extent_bits: series.max_extent_px.to_bits(),
                style_count: series.style_count,
                override_count: series.override_count,
                style_index_base: series.style_index_base,
                style_index_len: series.style_index_len,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    struct PickSeriesResourceSnapshot {
        query_params: wgpu::Buffer,
        gate_masks: wgpu::Buffer,
        workgroup_candidates: wgpu::Buffer,
        query_data_bg: wgpu::BindGroup,
        query_work_bg: wgpu::BindGroup,
        reduce_bg: wgpu::BindGroup,
        result: wgpu::Buffer,
    }

    impl PickSeriesResourceSnapshot {
        fn capture(slot: &PickSeriesSlot) -> Self {
            let series = &slot.gpu;
            Self {
                query_params: series.query_params.clone(),
                gate_masks: series.gate_masks.clone(),
                workgroup_candidates: series.workgroup_candidates.clone(),
                query_data_bg: series.query_data_bg.clone(),
                query_work_bg: series.query_work_bg.clone(),
                reduce_bg: series.reduce_bg.clone(),
                result: series.result.clone(),
            }
        }
    }

    struct PickEngineSnapshot {
        identities: Arc<[PickIdentity]>,
        metadata: Vec<PickSeriesMetadataSnapshot>,
        resources: Vec<PickSeriesResourceSnapshot>,
        reduce_candidates: wgpu::Buffer,
        reduce_final_result: wgpu::Buffer,
        reduce_bind_group: wgpu::BindGroup,
    }

    impl PickEngineSnapshot {
        fn capture(engine: &GpuPickEngine) -> Self {
            Self {
                identities: Arc::clone(&engine.registry.identities),
                metadata: engine
                    .registry
                    .slots
                    .iter()
                    .map(PickSeriesMetadataSnapshot::capture)
                    .collect(),
                resources: engine
                    .registry
                    .slots
                    .iter()
                    .map(PickSeriesResourceSnapshot::capture)
                    .collect(),
                reduce_candidates: engine.registry.reduce.candidates.clone(),
                reduce_final_result: engine.registry.reduce.final_result.clone(),
                reduce_bind_group: engine.registry.reduce.bind_group.clone(),
            }
        }

        fn assert_unchanged(&self, engine: &GpuPickEngine) {
            assert!(Arc::ptr_eq(&self.identities, &engine.registry.identities));
            assert_eq!(engine.registry.slots.len(), self.metadata.len());
            assert_eq!(
                engine
                    .registry
                    .slots
                    .iter()
                    .map(PickSeriesMetadataSnapshot::capture)
                    .collect::<Vec<_>>(),
                self.metadata
            );
            assert_eq!(
                engine
                    .registry
                    .slots
                    .iter()
                    .map(PickSeriesResourceSnapshot::capture)
                    .collect::<Vec<_>>(),
                self.resources
            );
            assert_eq!(engine.registry.reduce.candidates, self.reduce_candidates);
            assert_eq!(
                engine.registry.reduce.final_result,
                self.reduce_final_result
            );
            assert_eq!(engine.registry.reduce.bind_group, self.reduce_bind_group);
        }
    }

    #[test]
    fn shared_bundle_creates_pipelines_once_for_multiple_engines() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut events = Vec::new();
        let bundle = PickPipelineBundle::new_observed(
            Arc::clone(&device),
            Arc::clone(&queue),
            Arc::new(GpuLedger::new()),
            &mut |event| events.push(event),
        )
        .unwrap();
        let observed_stages = events
            .iter()
            .filter(|event| event.phase == InitPhase::Started)
            .map(|event| event.stage)
            .collect::<Vec<_>>();
        assert_eq!(
            observed_stages,
            [
                "setup",
                "pick_gate_x",
                "pick_gate_y",
                "pick_reduce_candidates",
            ]
        );
        let event_count = events.len();

        let first = GpuPickEngine::from_bundle(Arc::clone(&bundle)).unwrap();
        let second = GpuPickEngine::from_bundle(Arc::clone(&bundle)).unwrap();
        assert_eq!(events.len(), event_count);
        assert!(Arc::ptr_eq(&first.bundle, &second.bundle));
        assert_eq!(first.bundle.gate_x, second.bundle.gate_x);
        assert_eq!(first.bundle.gate_y, second.bundle.gate_y);
        assert_eq!(first.bundle.reduce, second.bundle.reduce);
    }

    #[test]
    fn streamed_exact_matches_resident_at_seams_with_global_styles_and_ties() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let slots = [ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [2.0, 1.0, 6.0, 0.0],
        }];
        let overrides = [ScatterStyleOverrideGpu {
            point_index: 5,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [8.0, 2.0, 6.0, 0.0],
        }];
        let descriptor = |order: usize, scatter: bool| GpuPickSeriesDescriptor {
            source_id: Some("stream-source".into()),
            series_id: format!("stream-{order}"),
            x_column: "x".into(),
            y_column: "y".into(),
            scatter: scatter.then_some(GpuPickScatter {
                base_radius_px: 1.0,
                base_shape_id: 5,
                style_map: Some(GpuPickScatterStyle {
                    style_index_column: Some("style".into()),
                    style_slots: &slots,
                    style_overrides: &overrides,
                    style_meta: ScatterStyleMapMeta {
                        style_count: 1,
                        override_count: 1,
                        has_index: 1,
                        _pad: 0,
                    },
                }),
                size_map: None,
            }),
            line_width_px: Some(2.0),
            line_interpolation: crate::data_config::LineInterpolation::Linear,
            line_style: crate::line::LineStylePreset::Solid,
            vector: None,
        };
        for logarithmic in [false, true] {
            let origin = if logarithmic {
                0.0
            } else {
                1_000_000_000_000.0
            };
            let xs = [1.0, 2.0, 4.0, f64::NAN, 6.0, 8.0, 8.0, 9.0].map(|x| x + origin);
            let ys = [5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 5.0];
            let style = [0.0f32, f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
            let mut config = parity_config(100, 100, (origin + 1.0, origin + 10.0), (0.0, 10.0));
            config.bottom_x.scale = if logarithmic {
                AxisScale::Logarithmic
            } else {
                AxisScale::Linear
            };
            config.bottom_x.inverted = logarithmic;
            config.left_y.inverted = logarithmic;
            let mut pool = ColumnPool::new(
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                4096,
            )
            .unwrap();
            pool.add_hilo_column(
                "x".into(),
                &f64_column(xs.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool.add_hilo_column(
                "y".into(),
                &f64_column(ys.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool.add_column(
                "style".into(),
                &f32_column(style.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            for scatter in [false, true] {
                let mut resident =
                    GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
                for order in 0..2 {
                    resident
                        .add_series(&pool, descriptor(order, scatter))
                        .unwrap();
                }
                for cursor in [
                    [10.0, 50.0],
                    [35.0, 50.0],
                    [55.0, 50.0],
                    [80.0, 54.0],
                    [95.0, 50.0],
                ] {
                    let query = query_from_config(&config, cursor, 4.0);
                    let expected = resolve_pick(&resident, &pool, query);
                    for chunk_size in [1usize, 2, 3, 5] {
                        let identities = (0..2)
                            .map(|order| (Some("stream-source".into()), format!("stream-{order}")))
                            .collect();
                        let exact = PickExactPipelines::default()
                            .ensure(&resident.bundle, PickExactKey::STREAMED)
                            .unwrap()
                            .clone();
                        let mut stream = resident
                            .bundle
                            .begin_stream(query, 1.0, identities, exact)
                            .unwrap();
                        let mut charges = vec![stream.charge()];
                        // Reverse both series and chunk submission order: ties
                        // are source identities, never scheduling order.
                        for order in (0..2).rev() {
                            let starts = (0..xs.len()).step_by(chunk_size).collect::<Vec<_>>();
                            for start in starts.into_iter().rev() {
                                let owned = chunk_size.min(xs.len() - start);
                                let end = (start + owned + 1).min(xs.len());
                                let len = end - start;
                                let mut pairs = Vec::new();
                                for value in &xs[start..end] {
                                    let (hi, lo) = split_f64_to_f32_pair(*value);
                                    pairs.push([hi, lo]);
                                }
                                for value in &ys[start..end] {
                                    let (hi, lo) = split_f64_to_f32_pair(*value);
                                    pairs.push([hi, lo]);
                                }
                                for value in &style[start..end] {
                                    pairs.push([*value, 0.0]);
                                }
                                let buffer = charged_buffer_init(
                                    &ChargeTally::new(),
                                    &device,
                                    &wgpu::util::BufferInitDescriptor {
                                        label: Some("stream pick test bounded source"),
                                        contents: bytemuck::cast_slice(&pairs),
                                        usage: wgpu::BufferUsages::STORAGE,
                                    },
                                );
                                let handle = |column| ColumnHandle {
                                    generation: 0,
                                    offset: (column * len * 8) as u64,
                                    byte_size: (len * 8) as u64,
                                    len_values: len,
                                };
                                let mut encoder =
                                    device.create_command_encoder(&Default::default());
                                if let Some(charge) = stream
                                    .encode_chunk(
                                        &mut encoder,
                                        &buffer,
                                        descriptor(order, scatter),
                                        GpuStreamPickColumns {
                                            x: handle(0),
                                            y: handle(1),
                                            style_index: scatter.then(|| handle(2)),
                                            size_value: None,
                                            vector: [None; 3],
                                        },
                                        order as u32,
                                        start as u32,
                                        owned as u32,
                                        owned.min(xs.len() - start - 1) as u32,
                                    )
                                    .unwrap()
                                {
                                    charges.push(charge);
                                }
                                queue.submit([encoder.finish()]);
                            }
                        }
                        let actual =
                            pollster::block_on(stream.finish().unwrap().resolve()).unwrap();
                        assert_eq!(
                            expected, actual,
                            "stream parity logarithmic={logarithmic}, scatter={scatter}, cursor={cursor:?}, chunk_size={chunk_size}"
                        );
                        if let (Some(expected), Some(actual)) = (&expected, &actual) {
                            assert_eq!(
                                expected.distance_px.to_bits(),
                                actual.distance_px.to_bits()
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn streamed_gates_keep_boundary_segment_and_exclude_unowned_halo() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 2;
        let mut pairs = vec![[f32::NAN, 0.0]; count * 2];
        pairs[count - 2] = [1.0, 0.0];
        pairs[count - 1] = [9.0, 0.0];
        pairs[2 * count - 2] = [5.0, 0.0];
        pairs[2 * count - 1] = [5.0, 0.0];
        let buffer = charged_buffer_init(
            &ChargeTally::new(),
            &device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("stream gated pick source"),
                contents: bytemuck::cast_slice(&pairs),
                usage: wgpu::BufferUsages::STORAGE,
            },
        );
        let columns = GpuStreamPickColumns {
            x: ColumnHandle {
                generation: 0,
                offset: 0,
                byte_size: count as u64 * 8,
                len_values: count,
            },
            y: ColumnHandle {
                generation: 0,
                offset: count as u64 * 8,
                byte_size: count as u64 * 8,
                len_values: count,
            },
            style_index: None,
            size_value: None,
            vector: [None; 3],
        };
        let bundle = PickPipelineBundle::new_observed(
            Arc::clone(&device),
            Arc::clone(&queue),
            Arc::new(GpuLedger::new()),
            &mut |_| {},
        )
        .unwrap();
        // Exact centerline hits must survive reciprocal rounding; near misses
        // and points beyond either endpoint must not become infinite-line hits.
        // A zero-width line is drawn one pixel wide, so a quarter pixel off
        // the centreline is on the stroke and three quarters is not.
        // The right halo is available to the last segment, never to scatter.
        for (line, cursor, expected_local_index) in [
            (false, [90.0, 50.0], None),
            (false, [10.0, 50.0], Some(count - 2)),
            (true, [50.0, 50.0], Some(count - 2)),
            (true, [30.0, 50.0], Some(count - 2)),
            (true, [70.0, 50.0], Some(count - 1)),
            (true, [50.0, 50.25], Some(count - 2)),
            (true, [50.0, 50.75], None),
            (true, [5.0, 50.0], None),
            (true, [95.0, 50.0], None),
        ] {
            let descriptor = GpuPickSeriesDescriptor {
                source_id: None,
                series_id: "gated-stream".into(),
                x_column: "x".into(),
                y_column: "y".into(),
                scatter: (!line).then_some(GpuPickScatter {
                    base_radius_px: 2.0,
                    base_shape_id: 5,
                    style_map: None,
                    size_map: None,
                }),
                line_width_px: line.then_some(0.0),
                line_interpolation: crate::data_config::LineInterpolation::Linear,
                line_style: crate::line::LineStylePreset::Solid,
                vector: None,
            };
            let query = test_query(cursor);
            let exact = PickExactPipelines::default()
                .ensure(&bundle, PickExactKey::STREAMED)
                .unwrap()
                .clone();
            let mut stream = bundle
                .begin_stream(query, 1.0, vec![(None, "gated-stream".into())], exact)
                .unwrap();
            assert_eq!(
                stream.charge().charged_bytes(),
                GpuStreamPick::PERSISTENT_BYTES
            );
            let bytes = stream.chunk_bytes(&descriptor, count).unwrap();
            let mut encoder = device.create_command_encoder(&Default::default());
            let scratch = stream
                .encode_chunk(
                    &mut encoder,
                    &buffer,
                    descriptor,
                    columns,
                    0,
                    1000,
                    (count - 1) as u32,
                    (count - 1) as u32,
                )
                .unwrap()
                .unwrap();
            assert_eq!(scratch.charged_bytes(), bytes);
            queue.submit([encoder.finish()]);
            let answer = pollster::block_on(stream.finish().unwrap().resolve()).unwrap();
            if let Some(expected_local_index) = expected_local_index {
                let answer = answer.unwrap_or_else(|| {
                    panic!("owned primitive at cursor was missed: line={line}, cursor={cursor:?}")
                });
                assert_eq!(answer.point_index, 1000 + expected_local_index);
                assert_eq!(answer.distance_px, 0.0);
            } else {
                assert_eq!(
                    answer, None,
                    "unexpected hit: line={line}, cursor={cursor:?}"
                );
            }
        }
    }

    #[test]
    fn prepared_registry_drop_preserves_exact_state() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "transition-drop-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "transition-drop-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                scatter_descriptor("drop-old", "transition-drop-x", "transition-drop-y"),
            )
            .unwrap();
        let snapshot = PickEngineSnapshot::capture(&engine);

        let transition = engine
            .prepare_registry_transition(
                &pool,
                [GpuPickRegistrySlot::Build(scatter_descriptor(
                    "drop-next",
                    "transition-drop-x",
                    "transition-drop-y",
                ))],
            )
            .unwrap();
        drop(transition);

        snapshot.assert_unchanged(&engine);
        assert_eq!(
            resolve_pick(&engine, &pool, test_query([50.0, 50.0]))
                .unwrap()
                .series_id,
            "drop-old"
        );
    }

    #[test]
    fn source_identity_transition_reuses_every_gpu_resource() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "identity-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "identity-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                scatter_descriptor("identity-series", "identity-x", "identity-y"),
            )
            .unwrap();
        let resources = PickSeriesResourceSnapshot::capture(&engine.registry.slots[0]);
        let reduce = engine.registry.reduce.clone();

        engine
            .prepare_registry_transition(
                &pool,
                [GpuPickRegistrySlot::Reuse {
                    current_index: 0,
                    source_id: Some("identity-source-next".into()),
                    series_id: "identity-series".into(),
                }],
            )
            .unwrap()
            .commit();

        assert_eq!(
            PickSeriesResourceSnapshot::capture(&engine.registry.slots[0]),
            resources
        );
        assert_eq!(engine.registry.reduce.candidates, reduce.candidates);
        assert_eq!(engine.registry.reduce.final_result, reduce.final_result);
        assert_eq!(engine.registry.reduce.bind_group, reduce.bind_group);
        let picked = resolve_pick(&engine, &pool, test_query([50.0, 50.0])).unwrap();
        assert_eq!(picked.source_id.as_deref(), Some("identity-source-next"));
        assert_eq!(picked.series_id, "identity-series");
    }

    #[test]
    fn foreign_pool_with_matching_numeric_stamps_is_rejected() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let make_pool = || {
            let mut pool = ColumnPool::new(
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                64 * 1024,
            )
            .unwrap();
            pool.add_column(
                "foreign-x".into(),
                &f32_column(vec![5.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool.add_column(
                "foreign-y".into(),
                &f32_column(vec![5.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
            pool
        };
        let first_pool = make_pool();
        let second_pool = make_pool();
        assert_eq!(
            first_pool.layout_generation(),
            second_pool.layout_generation()
        );
        assert_eq!(
            first_pool.allocation_epoch("foreign-x"),
            second_pool.allocation_epoch("foreign-x")
        );
        assert_eq!(
            handle_snapshot(first_pool.handle_for("foreign-x").unwrap()),
            handle_snapshot(second_pool.handle_for("foreign-x").unwrap())
        );

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &first_pool,
                scatter_descriptor("foreign", "foreign-x", "foreign-y"),
            )
            .unwrap();
        assert!(matches!(
            engine.pick(&second_pool, test_query([50.0, 50.0])),
            Err(GpuPickError::ForeignColumnPool)
        ));
        let transition = engine.prepare_registry_transition(
            &second_pool,
            [GpuPickRegistrySlot::Reuse {
                current_index: 0,
                source_id: None,
                series_id: "foreign".into(),
            }],
        );
        assert!(matches!(transition, Err(GpuPickError::ForeignColumnPool)));
    }

    #[test]
    fn reduce_resources_grow_only_when_live_count_exceeds_capacity() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "grow-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "grow-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        for id in ["grow-a", "grow-b"] {
            engine
                .add_series(&pool, scatter_descriptor(id, "grow-x", "grow-y"))
                .unwrap();
        }
        assert_eq!(engine.registry.reduce.capacity, 2);
        let at_two = engine.registry.reduce.candidates.clone();

        engine
            .replace_series_at(0, &pool, scatter_descriptor("grow-a2", "grow-x", "grow-y"))
            .unwrap();
        assert_eq!(engine.registry.reduce.candidates, at_two);
        engine
            .prepare_registry_transition(
                &pool,
                [
                    GpuPickRegistrySlot::Reuse {
                        current_index: 1,
                        source_id: None,
                        series_id: "grow-b".into(),
                    },
                    GpuPickRegistrySlot::Reuse {
                        current_index: 0,
                        source_id: None,
                        series_id: "grow-a2".into(),
                    },
                ],
            )
            .unwrap()
            .commit();
        assert_eq!(engine.registry.reduce.candidates, at_two);
        engine.remove_series_at(1).unwrap();
        assert_eq!(engine.registry.reduce.capacity, 2);
        assert_eq!(engine.registry.reduce.candidates, at_two);
        engine
            .add_series(&pool, scatter_descriptor("grow-c", "grow-x", "grow-y"))
            .unwrap();
        assert_eq!(engine.registry.reduce.candidates, at_two);
        engine
            .add_series(&pool, scatter_descriptor("grow-d", "grow-x", "grow-y"))
            .unwrap();
        assert_eq!(engine.registry.reduce.capacity, 3);
        assert_ne!(engine.registry.reduce.candidates, at_two);
    }

    #[test]
    fn exact_gpu_scatter_style_mapping_matches_cpu_exhaustive_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
        let xs = [2.0, 5.0, 8.0];
        let ys = [5.0, 5.0, 5.0];
        let style_indices = [1.0, 0.0, 99.0];
        let cpu_columns = CpuColumns::new(&[
            ("parity-sx", &xs),
            ("parity-sy", &ys),
            ("parity-si", &style_indices),
        ]);

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, values) in [
            ("parity-sx", xs.as_slice()),
            ("parity-sy", ys.as_slice()),
            ("parity-si", style_indices.as_slice()),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }

        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 1.0,
            point_style_table: Some(vec![
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::SquareFilled),
                    point_size: Some(2.0),
                },
                DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::TriangleFilled),
                    point_size: Some(3.0),
                },
            ]),
            point_style_index_column: Some("parity-si".into()),
            point_style_overrides: Some(vec![DataScatterPointStyleOverride {
                index: 1,
                style: DataScatterPointStyleConfig {
                    point_color: None,
                    point_shape: Some(ScatterShape::DiamondFilled),
                    point_size: Some(5.0),
                },
            }]),
            point_color_map: None,
            point_size_map: None,
        };
        let cpu_series = [scatter_series(
            "mapped",
            Some("mapped-source"),
            "parity-sx",
            "parity-sy",
            scatter,
        )];
        let slots = [
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [
                    2.0,
                    crate::data_render::shape_id(&ScatterShape::SquareFilled) as f32,
                    (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                    0.0,
                ],
            },
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [
                    3.0,
                    crate::data_render::shape_id(&ScatterShape::TriangleFilled) as f32,
                    (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                    0.0,
                ],
            },
        ];
        let overrides = [ScatterStyleOverrideGpu {
            point_index: 1,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [
                5.0,
                crate::data_render::shape_id(&ScatterShape::DiamondFilled) as f32,
                (STYLE_MASK_RADIUS | STYLE_MASK_SHAPE) as f32,
                0.0,
            ],
        }];
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("mapped-source".into()),
                    series_id: "mapped".into(),
                    x_column: "parity-sx".into(),
                    y_column: "parity-sy".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: Some(GpuPickScatterStyle {
                            style_index_column: Some("parity-si".into()),
                            style_slots: &slots,
                            style_overrides: &overrides,
                            style_meta: ScatterStyleMapMeta {
                                style_count: slots.len() as u32,
                                override_count: overrides.len() as u32,
                                has_index: 1,
                                _pad: 0,
                            },
                        }),
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        // Point 0 is a filled triangle of visual radius 3 * 1.555 with its
        // apex up; 3 px above its centre is inside it, while the disc the
        // old picker used would also have taken 4.5 px to the right, where
        // the triangle draws nothing. Point 1 is a filled diamond of visual
        // radius 5 * 1.253 = 6.27, so 6 px right of its centre is inside.
        for (cursor, expected_index) in [([20.0, 47.0], 0), ([56.0, 50.0], 1)] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }

        // Beside the triangle, inside its circumscribed disc, nothing is
        // drawn and nothing is picked.
        let cursor = [24.5, 50.0];
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
        assert!(cpu.is_none(), "{cpu:?}");
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu, gpu);

        // An out-of-range style-table index falls back to the one-pixel base
        // circle; 1.25 px outside its center must therefore miss exactly.
        let cursor = [81.25, 50.0];
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu, gpu);
    }

    /// A `point_size_map` resolves each point's pick radius from its value on
    /// both pickers: the domain ends map to `min_px` / `max_px`, and a
    /// non-finite value hides the point from both.
    #[test]
    fn size_map_radius_parity_between_cpu_and_gpu_pickers() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(200, 100, (0.0, 20.0), (0.0, 10.0));
        let xs = [2.0, 10.0, 18.0];
        let ys = [5.0, 5.0, 5.0];
        let sizes = [0.0, 10.0, f32::NAN];
        let cpu_columns = CpuColumns::new(&[("size-x", &xs), ("size-y", &ys), ("size-s", &sizes)]);

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, values) in [
            ("size-x", xs.as_slice()),
            ("size-y", ys.as_slice()),
            ("size-s", sizes.as_slice()),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }

        let size_map = crate::data_config::PointSizeMap {
            column: "size-s".into(),
            domain_min: 0.0,
            domain_max: 10.0,
            min_px: 1.0,
            max_px: 5.0,
            scale: crate::data_config::SizeScale::Radius,
        };
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 3.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: Some(size_map.clone()),
        };
        let cpu_series = [scatter_series("sized", None, "size-x", "size-y", scatter)];
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "sized".into(),
                    x_column: "size-x".into(),
                    y_column: "size-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 3.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: None,
                        size_map: Some(GpuPickSizeMap {
                            column: size_map.column.clone(),
                            domain_min: size_map.domain_min,
                            domain_max: size_map.domain_max,
                            min_px: size_map.min_px,
                            max_px: size_map.max_px,
                            area: false,
                        }),
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        // Point 0 at 20 px with a 1 px radius, point 1 at 100 px with 5 px.
        // Every hit sits strictly inside the 1 px pick distance: a cursor on
        // the boundary itself depends on backend rounding, which the exact
        // pass does not pad.
        for (cursor, expected) in [
            ([21.5, 50.0], Some((0, 0.5))),
            ([24.5, 50.0], None),
            ([105.5, 50.0], Some((1, 0.5))),
            ([107.5, 50.0], None),
            // Point 2 is hidden by its NaN size: nothing to hit at its center.
            ([180.0, 50.0], None),
        ] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 1.0);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 1.0));
            assert_pick_parity(cpu.clone(), gpu);
            match expected {
                Some((index, distance)) => {
                    let cpu = cpu.expect("a hit");
                    assert_eq!(cpu.point_index, index);
                    assert!((cpu.distance_px - distance).abs() < 1e-3);
                }
                None => assert!(cpu.is_none(), "unexpected hit at {cursor:?}: {cpu:?}"),
            }
        }
    }

    #[test]
    fn display_scale_applies_once_to_pick_geometry_but_not_pick_distance() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(200, 100, (0.0, 20.0), (0.0, 10.0));
        let base_x = [2.0];
        let base_y = [5.0];
        let mapped_x = [10.0];
        let mapped_y = [5.0];
        let mapped_style = [0.0];
        let line_x = [15.0, 17.0];
        let line_y = [2.0, 2.0];

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, values) in [
            ("scale-base-x", base_x.as_slice()),
            ("scale-base-y", base_y.as_slice()),
            ("scale-mapped-x", mapped_x.as_slice()),
            ("scale-mapped-y", mapped_y.as_slice()),
            ("scale-style", mapped_style.as_slice()),
            ("scale-line-x", line_x.as_slice()),
            ("scale-line-y", line_y.as_slice()),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }

        let mapped_slots = [ScatterStyleSlotGpu {
            color_premul: [0.0; 4],
            meta: [6.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
        }];
        let mapped_overrides = [ScatterStyleOverrideGpu {
            point_index: 0,
            _pad: [0; 3],
            color_premul: [0.0; 4],
            meta: [
                0.0,
                crate::data_render::shape_id(&ScatterShape::Triangle) as f32,
                STYLE_MASK_SHAPE as f32,
                0.0,
            ],
        }];
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "scaled-base".into(),
                    x_column: "scale-base-x".into(),
                    y_column: "scale-base-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 4.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "scaled-mapped".into(),
                    x_column: "scale-mapped-x".into(),
                    y_column: "scale-mapped-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::Circle),
                        style_map: Some(GpuPickScatterStyle {
                            style_index_column: Some("scale-style".into()),
                            style_slots: &mapped_slots,
                            style_overrides: &mapped_overrides,
                            style_meta: ScatterStyleMapMeta {
                                style_count: 1,
                                override_count: 1,
                                has_index: 1,
                                _pad: 0,
                            },
                        }),
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "scaled-line".into(),
                    x_column: "scale-line-x".into(),
                    y_column: "scale-line-y".into(),
                    scatter: None,
                    line_width_px: Some(6.0),
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        for display_scale in [0.5, 1.0, 2.0] {
            // The base marker is an outline circle: a stroke of half-width
            // `STROKE_HALF` (physical pixels, unscaled) centred on the scaled
            // radius, hollow inside.
            let base_radius = 4.0 * display_scale;
            let on_stroke_inside = query_from_config(
                &config,
                [20.0 + base_radius - STROKE_HALF + 0.25, 50.0],
                0.0,
            );
            let on_stroke_outside = query_from_config(
                &config,
                [20.0 + base_radius + STROKE_HALF - 0.25, 50.0],
                0.0,
            );
            let base_outside = query_from_config(
                &config,
                [20.0 + base_radius + STROKE_HALF + 0.25, 50.0],
                0.0,
            );
            for hit in [on_stroke_inside, on_stroke_outside] {
                assert_eq!(
                    resolve_pick_scaled(&engine, &pool, hit, display_scale)
                        .unwrap()
                        .series_id,
                    "scaled-base"
                );
            }
            assert!(resolve_pick_scaled(&engine, &pool, base_outside, display_scale).is_none());
            if base_radius > STROKE_HALF + 0.5 {
                let hollow = query_from_config(&config, [20.0, 50.0], 0.0);
                assert!(
                    resolve_pick_scaled(&engine, &pool, hollow, display_scale).is_none(),
                    "the hollow centre of an outline circle draws nothing"
                );
            }

            // The mapped marker is an outline triangle of reference radius 6:
            // its apex sits straight up (canvas y decreasing) at the visual
            // radius.
            let mapped_radius = 6.0 * MAX_SHAPE_SCALE * display_scale;
            let mapped_inside =
                query_from_config(&config, [100.0, 50.0 - mapped_radius + 0.25], 0.0);
            let mapped_outside = query_from_config(
                &config,
                [100.0, 50.0 - mapped_radius - STROKE_HALF - 0.25],
                0.0,
            );
            assert_eq!(
                resolve_pick_scaled(&engine, &pool, mapped_inside, display_scale)
                    .unwrap()
                    .series_id,
                "scaled-mapped"
            );
            assert!(resolve_pick_scaled(&engine, &pool, mapped_outside, display_scale).is_none());

            let line_half_width = 3.0 * display_scale;
            let line_inside =
                query_from_config(&config, [160.0, 80.0 + line_half_width - 0.25], 0.0);
            let line_outside =
                query_from_config(&config, [160.0, 80.0 + line_half_width + 0.25], 0.0);
            assert_eq!(
                resolve_pick_scaled(&engine, &pool, line_inside, display_scale)
                    .unwrap()
                    .series_id,
                "scaled-line"
            );
            assert!(resolve_pick_scaled(&engine, &pool, line_outside, display_scale).is_none());

            let physical_gap = 2.5;
            let edge = base_radius + STROKE_HALF;
            let max_distance_miss =
                query_from_config(&config, [20.0 + edge + physical_gap, 50.0], 2.0);
            let max_distance_hit =
                query_from_config(&config, [20.0 + edge + physical_gap, 50.0], 3.0);
            assert!(
                resolve_pick_scaled(&engine, &pool, max_distance_miss, display_scale).is_none()
            );
            let picked =
                resolve_pick_scaled(&engine, &pool, max_distance_hit, display_scale).unwrap();
            assert_eq!(picked.series_id, "scaled-base");
            assert!((picked.distance_px - physical_gap).abs() <= 0.001);
        }
    }

    #[test]
    fn invalid_display_scales_fail_closed_without_gpu_submission() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "invalid-scale-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "invalid-scale-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                scatter_descriptor("invalid-scale", "invalid-scale-x", "invalid-scale-y"),
            )
            .unwrap();

        for display_scale in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                resolve_pick_scaled(&engine, &pool, test_query([50.0, 50.0]), display_scale,)
                    .is_none()
            );
        }
    }

    #[test]
    fn exact_gpu_line_boundary_endpoint_matches_cpu_exhaustive_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(650, 100, (0.0, 65.0), (0.0, 10.0));
        let xs = (0..66).map(|value| value as f32).collect::<Vec<_>>();
        let ys = vec![5.0; xs.len()];
        let cpu_columns =
            CpuColumns::new(&[("parity-lx", xs.as_slice()), ("parity-ly", ys.as_slice())]);
        let cpu_series = [line_series(
            "boundary-line",
            Some("line-source"),
            "parity-lx",
            "parity-ly",
            0.0,
        )];

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "parity-lx".into(),
            &f32_column(xs),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "parity-ly".into(),
            &f32_column(ys),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("line-source".into()),
                    series_id: "boundary-line".into(),
                    x_column: "parity-lx".into(),
                    y_column: "parity-ly".into(),
                    scatter: None,
                    line_width_px: Some(0.0),
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        // Segment start 63 and endpoint 64 straddle two 32-index gate words.
        // The start word must still retain and exact-test that segment.
        for (cursor, expected_index) in [
            ([634.998, 50.0], 63),
            ([635.0, 50.0], 63),
            ([635.002, 50.0], 64),
            ([637.5, 50.0], 64),
        ] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.01);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.01));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }
    }

    #[test]
    fn exact_gpu_line_gate_keeps_a_segment_whose_endpoints_are_both_outside() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
        let xs = [0.0, 10.0];
        let ys = [0.0, 10.0];
        let cpu_columns = CpuColumns::new(&[
            ("gate-line-x", xs.as_slice()),
            ("gate-line-y", ys.as_slice()),
        ]);
        let cpu_series = [line_series(
            "gate-line",
            None,
            "gate-line-x",
            "gate-line-y",
            0.0,
        )];

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "gate-line-x".into(),
            &f32_column(xs.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "gate-line-y".into(),
            &f32_column(ys.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "gate-line".into(),
                    x_column: "gate-line-x".into(),
                    y_column: "gate-line-y".into(),
                    scatter: None,
                    line_width_px: Some(0.0),
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        let cursor = [50.0, 50.0];
        let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.01);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.01));
        assert_pick_parity(cpu, gpu);
    }

    #[test]
    fn two_dimensional_gate_dispatch_returns_the_original_index() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let point_count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 1;
        let xs = (0..point_count)
            .map(|index| ((index * 2_053) % point_count) as f32)
            .collect::<Vec<_>>();
        let ys = (0..point_count)
            .map(|index| ((index * 12_337) % point_count) as f32)
            .collect::<Vec<_>>();
        let target_index = point_count / 2;
        let axis_max = (point_count - 1) as f32;
        let cursor = [
            xs[target_index] / axis_max * 100.0,
            (1.0 - ys[target_index] / axis_max) * 100.0,
        ];
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            2 * 1024 * 1024,
        )
        .unwrap();
        pool.add_column(
            "gate-2d-x".into(),
            &f32_column(xs),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "gate-2d-y".into(),
            &f32_column(ys),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "gate-2d".into(),
                    x_column: "gate-2d-x".into(),
                    y_column: "gate-2d-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 0.0001,
                        base_shape_id: 5,
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        let groups = engine.registry.slots[0]
            .gpu
            .gate_word_count
            .div_ceil(GPU_PICK_WORKGROUP_SIZE);
        assert_eq!(groups, 33);
        assert!(!engine.registry.slots[0].gpu.direct_scan);
        engine.registry.slots[0].gpu.dispatch_x = 1;
        engine.registry.slots[0].gpu.dispatch_y = groups;
        let picked = resolve_pick(
            &engine,
            &pool,
            GpuPickQuery {
                transform: ScatterTransform {
                    data_min: [0.0, 0.0],
                    data_max: [axis_max, axis_max],
                    data_min_lo: [0.0; 2],
                    data_max_lo: [0.0; 2],
                    scale_log: [0.0; 2],
                    pixel_to_ndc: [0.02; 2],
                    data_to_panel_scale: [1.0; 2],
                    data_to_panel_offset: [0.0; 2],
                    style_params: [[0.0; 4]; 3],
                    polar_angle: [0.0; 4],
                    polar_extent: [0.0; 4],
                },
                chart_rect_px: [0.0, 0.0, 100.0, 100.0],
                data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
                canvas_position_px: cursor,
                max_distance_px: 0.0,
                draw_style: GpuPickDrawStyle::PRECISE,
            },
        )
        .unwrap();
        assert_eq!(picked.point_index, target_index);
        assert_eq!(picked.distance_px, 0.0);
    }

    #[test]
    fn overlapping_tickets_resolve_after_registry_engine_and_pool_drop() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let point_count = GPU_PICK_DIRECT_SCAN_POINTS as usize + 1;
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            2 * 1024 * 1024,
        )
        .unwrap();
        pool.add_column(
            "overlap-x".into(),
            &f32_column((0..point_count).map(|index| index as f32).collect()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "overlap-y".into(),
            &f32_column(vec![5.0; point_count]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("overlap-source".into()),
                    series_id: "overlap-series".into(),
                    x_column: "overlap-x".into(),
                    y_column: "overlap-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 0.0001,
                        base_shape_id: 5,
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();
        assert!(!engine.registry.slots[0].gpu.direct_scan);

        let query = |cursor_x| GpuPickQuery {
            transform: ScatterTransform {
                data_min: [0.0, 0.0],
                data_max: [(point_count - 1) as f32, 10.0],
                data_min_lo: [0.0; 2],
                data_max_lo: [0.0; 2],
                scale_log: [0.0; 2],
                pixel_to_ndc: [0.02; 2],
                data_to_panel_scale: [1.0; 2],
                data_to_panel_offset: [0.0; 2],
                style_params: [[0.0; 4]; 3],
                polar_angle: [0.0; 4],
                polar_extent: [0.0; 4],
            },
            chart_rect_px: [0.0, 0.0, 100.0, 100.0],
            data_area_px: Some([0.0, 0.0, 100.0, 100.0]),
            canvas_position_px: [cursor_x, 50.0],
            max_distance_px: 0.0,
            draw_style: GpuPickDrawStyle::PRECISE,
        };
        let left = engine.pick(&pool, query(25.0)).unwrap();
        let right = engine.pick(&pool, query(75.0)).unwrap();
        engine.clear_series();
        drop(engine);
        drop(pool);

        let right = pollster::block_on(right.resolve()).unwrap().unwrap();
        let left = pollster::block_on(left.resolve()).unwrap().unwrap();
        assert_eq!(right.source_id.as_deref(), Some("overlap-source"));
        assert_eq!(right.series_id, "overlap-series");
        assert_eq!(right.point_index, (point_count - 1) * 3 / 4);
        assert_eq!(right.distance_px, 0.0);
        assert_eq!(left.source_id.as_deref(), Some("overlap-source"));
        assert_eq!(left.series_id, "overlap-series");
        assert_eq!(left.point_index, (point_count - 1) / 4);
        assert_eq!(left.distance_px, 0.0);
    }

    #[test]
    fn exact_gpu_log_inverted_and_nonfinite_semantics_match_cpu_picker() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut config = parity_config(120, 100, (1.0, 1_000.0), (0.0, 10.0));
        config.bottom_x.scale = AxisScale::Logarithmic;
        config.left_y.inverted = true;
        let xs = [f32::NAN, f32::INFINITY, 0.0, -1.0, 10.0, 100.0];
        let ys = [5.0, 5.0, 5.0, 5.0, 2.0, 8.0];
        let cpu_columns = CpuColumns::new(&[("parity-log-x", &xs), ("parity-log-y", &ys)]);
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 2.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        };
        let cpu_series = [scatter_series(
            "log-inverted",
            None,
            "parity-log-x",
            "parity-log-y",
            scatter,
        )];

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "parity-log-x".into(),
            &f32_column(xs.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "parity-log-y".into(),
            &f32_column(ys.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "log-inverted".into(),
                    x_column: "parity-log-x".into(),
                    y_column: "parity-log-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 2.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        for (cursor, expected_index) in [([40.0, 20.0], 4), ([80.0, 80.0], 5)] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, 0.0);
            let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
            assert_pick_parity(cpu.clone(), gpu);
            assert_eq!(cpu.unwrap().point_index, expected_index);
        }

        // Invalid samples must not become pickable at the old 1e-30 clamp,
        // even when that artificial coordinate lies inside the visible range.
        let mut tiny = config.clone();
        tiny.bottom_x.min = 1e-31;
        tiny.bottom_x.max = 1e-29;
        let cursor = [60.0, 50.0];
        assert!(cpu_pick(&tiny, &cpu_series, &cpu_columns, cursor, 2.0).is_none());
        assert!(resolve_pick(&engine, &pool, query_from_config(&tiny, cursor, 2.0)).is_none());

        for (cursor, max_distance) in [
            ([f32::NAN, 20.0], 1.0),
            ([40.0, f32::INFINITY], 1.0),
            ([40.0, 20.0], f32::NAN),
            ([40.0, 20.0], -1.0),
            ([-1.0, 20.0], 1.0),
        ] {
            let cpu = cpu_pick(&config, &cpu_series, &cpu_columns, cursor, max_distance);
            let gpu = resolve_pick(
                &engine,
                &pool,
                query_from_config(&config, cursor, max_distance),
            );
            assert_pick_parity(cpu, gpu);
        }
    }

    /// A stepped line is picked along its axis-aligned pieces on both
    /// pickers: hits on a run, on a riser and near the corners resolve to the
    /// same source index at the same distance, on linear and logarithmic
    /// axes, and the straight chord between two samples is not pickable.
    #[test]
    fn exact_gpu_step_lines_match_cpu_picker_on_runs_risers_and_corners() {
        use crate::data_config::LineInterpolation;

        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        // Three samples: a rising step, then a falling one with a long riser.
        let xs = [1.0f32, 3.0, 7.0];
        let ys = [1.0f32, 5.0, 2.0];
        let cpu_columns = CpuColumns::new(&[
            ("parity-step-x", xs.as_slice()),
            ("parity-step-y", ys.as_slice()),
        ]);
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "parity-step-x".into(),
            &f32_column(xs.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "parity-step-y".into(),
            &f32_column(ys.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

        for logarithmic in [false, true] {
            let mut config = parity_config(400, 400, (1.0, 8.0), (1.0, 8.0));
            if logarithmic {
                config.bottom_x.scale = AxisScale::Logarithmic;
                config.left_y.scale = AxisScale::Logarithmic;
            }
            // Cursor positions in canvas pixels, from data coordinates through
            // the chart's own data area and scale.
            let to_px = |x: f32, y: f32| -> [f32; 2] {
                let area = config.data_area().unwrap().0;
                let t = |v: f32, min: f64, max: f64| {
                    if logarithmic {
                        (f64::from(v).log10() - min.log10()) / (max.log10() - min.log10())
                    } else {
                        (f64::from(v) - min) / (max - min)
                    }
                };
                let tx = t(x, config.bottom_x.min, config.bottom_x.max);
                let ty = t(y, config.left_y.min, config.left_y.max);
                [
                    area.x as f32 + tx as f32 * area.width as f32,
                    (area.y + area.height) as f32 - ty as f32 * area.height as f32,
                ]
            };
            for mode in [
                LineInterpolation::Linear,
                LineInterpolation::StepPre,
                LineInterpolation::StepPost,
                LineInterpolation::StepMid,
            ] {
                let mut cpu_series = line_series(
                    "step",
                    Some("step-source"),
                    "parity-step-x",
                    "parity-step-y",
                    3.0,
                );
                let DataRenderType::Line { line } = &mut cpu_series.render_type else {
                    unreachable!()
                };
                line.interpolation = mode;
                let cpu_series = [cpu_series];
                let mut engine =
                    GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
                engine
                    .add_series(
                        &pool,
                        GpuPickSeriesDescriptor {
                            source_id: Some("step-source".into()),
                            series_id: "step".into(),
                            x_column: "parity-step-x".into(),
                            y_column: "parity-step-y".into(),
                            scatter: None,
                            line_width_px: Some(3.0),
                            line_interpolation: mode,
                            line_style: crate::line::LineStylePreset::Solid,
                            vector: None,
                        },
                    )
                    .unwrap();
                // Data-space probes: on the first run / riser, on the long
                // falling riser of the second segment, just past the step-mid
                // midpoint, and on the chord between samples 0 and 1 (which
                // only the linear line touches).
                let probes = [
                    (2.0f32, 1.0f32),
                    (1.0, 3.0),
                    (3.0, 3.0),
                    (2.0, 5.0),
                    (2.0, 3.0),
                    (5.0, 4.5),
                    (5.0, 2.5),
                    (7.0, 3.5),
                    (3.0, 4.0),
                    (5.2, 5.0),
                    (4.8, 2.0),
                    (6.9, 4.9),
                ];
                let mut hits = 0;
                for (x, y) in probes {
                    for max_distance in [0.75, 6.0, 40.0] {
                        let cursor = to_px(x, y);
                        let cpu =
                            cpu_pick(&config, &cpu_series, &cpu_columns, cursor, max_distance);
                        let gpu = resolve_pick(
                            &engine,
                            &pool,
                            query_from_config(&config, cursor, max_distance),
                        );
                        hits += usize::from(cpu.is_some());
                        assert_pick_parity(cpu, gpu);
                    }
                }
                assert!(
                    hits >= 3,
                    "{mode:?} log={logarithmic}: too few hits ({hits}) to be meaningful"
                );
                // Mode-specific resolution on a linear axis, checked on the
                // CPU side (the GPU side agreed above).
                if !logarithmic {
                    let pick = |x: f32, y: f32| {
                        cpu_pick(&config, &cpu_series, &cpu_columns, to_px(x, y), 0.75)
                            .map(|p| p.point_index)
                    };
                    match mode {
                        LineInterpolation::Linear => {
                            assert_eq!(
                                pick(2.0, 3.0),
                                Some(0),
                                "linear: the exact midpoint ties to the lower index"
                            );
                            assert_eq!(pick(2.5, 4.0), Some(1), "linear: on the chord, nearer B");
                            assert_eq!(pick(2.0, 1.0), None, "linear: no run at y_a");
                        }
                        LineInterpolation::StepPost => {
                            assert_eq!(pick(2.0, 1.0), Some(0), "post: on the run at y_a");
                            assert_eq!(pick(3.0, 4.0), Some(1), "post: high on the riser at x_b");
                            assert_eq!(pick(2.0, 3.0), None, "post: the chord is not drawn");
                        }
                        LineInterpolation::StepPre => {
                            assert_eq!(pick(1.0, 3.0), Some(0), "pre: low on the riser at x_a");
                            assert_eq!(pick(2.0, 5.0), Some(1), "pre: on the run at y_b");
                            assert_eq!(pick(2.0, 3.0), None, "pre: the chord is not drawn");
                        }
                        LineInterpolation::StepMid => {
                            assert_eq!(
                                pick(2.0, 3.0),
                                Some(0),
                                "mid: the riser's midpoint ties to the lower index"
                            );
                            assert_eq!(
                                pick(4.8, 5.0),
                                Some(1),
                                "mid: the run at y_a before the midpoint"
                            );
                            assert_eq!(
                                pick(5.2, 2.0),
                                Some(2),
                                "mid: the run at y_b after the midpoint"
                            );
                            assert_eq!(
                                pick(5.2, 5.0),
                                None,
                                "mid: nothing at y_a after the midpoint"
                            );
                            assert_eq!(
                                pick(4.8, 2.0),
                                None,
                                "mid: nothing at y_b before the midpoint"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn exact_gpu_equal_distance_ties_match_cpu_series_and_primitive_order() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let config = parity_config(100, 100, (0.0, 10.0), (0.0, 10.0));
        let first_xs = [4.0, 6.0];
        let first_ys = [5.0, 5.0];
        let later_xs = [5.6];
        let later_ys = [5.0];
        let cpu_columns = CpuColumns::new(&[
            ("parity-tie-x0", &first_xs),
            ("parity-tie-y0", &first_ys),
            ("parity-tie-x1", &later_xs),
            ("parity-tie-y1", &later_ys),
        ]);
        let scatter = DataScatterStyleConfig {
            point_color: Color::BLACK,
            point_shape: ScatterShape::CircleFilled,
            point_size: 20.0,
            point_style_table: None,
            point_style_index_column: None,
            point_style_overrides: None,
            point_color_map: None,
            point_size_map: None,
        };
        let line = DataLineStyleConfig {
            line_style: LineStylePreset::Solid,
            line_color: Color::BLACK,
            line_width: 0.0,
            fill: None,
            fill_color: None,
            interpolation: crate::data_config::LineInterpolation::Linear,
        };
        let first_series = SeriesConfig {
            series_id: "first".into(),
            source_id: Some("first-source".into()),
            label: None,
            x_column: "parity-tie-x0".into(),
            y_column: "parity-tie-y0".into(),
            render_type: DataRenderType::ScatterLine { scatter, line },
        };
        let later_series = scatter_series(
            "later",
            Some("later-source"),
            "parity-tie-x1",
            "parity-tie-y1",
            DataScatterStyleConfig {
                point_color: Color::BLACK,
                point_shape: ScatterShape::CircleFilled,
                point_size: 1.0,
                point_style_table: None,
                point_style_index_column: None,
                point_style_overrides: None,
                point_color_map: None,
                point_size_map: None,
            },
        );

        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, values) in [
            ("parity-tie-x0", first_xs.as_slice()),
            ("parity-tie-y0", first_ys.as_slice()),
            ("parity-tie-x1", later_xs.as_slice()),
            ("parity-tie-y1", later_ys.as_slice()),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values.to_vec()),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("first-source".into()),
                    series_id: "first".into(),
                    x_column: "parity-tie-x0".into(),
                    y_column: "parity-tie-y0".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 20.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: Some(0.0),
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        // Both scatter points and the line segment have distance zero at this
        // cursor. The topmost paint wins the tie: markers paint over their
        // series' line and instance 1 paints over instance 0, so both pickers
        // answer marker 1 (the old rule answered the covered marker 0).
        let cursor = [56.0, 50.0];
        let cpu = cpu_pick(
            &config,
            std::slice::from_ref(&first_series),
            &cpu_columns,
            cursor,
            0.0,
        );
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().point_index, 1);

        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("later-source".into()),
                    series_id: "later".into(),
                    x_column: "parity-tie-x1".into(),
                    y_column: "parity-tie-y1".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();
        let series = [first_series, later_series];
        let cpu = cpu_pick(&config, &series, &cpu_columns, cursor, 0.0);
        let gpu = resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0));
        assert_pick_parity(cpu.clone(), gpu);
        assert_eq!(cpu.unwrap().series_id, "later");
    }

    #[test]
    fn exact_gpu_f64_residual_selects_point_index() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let epoch = 1_700_000_000_000.0_f64;
        let xs = [epoch + 0.25, epoch + 0.75];
        let ys = [0.5_f64, 0.5];
        let (x0_hi, x0_lo) = split_f64_to_f32_pair(xs[0]);
        let (x1_hi, x1_lo) = split_f64_to_f32_pair(xs[1]);
        assert_eq!(x0_hi.to_bits(), x1_hi.to_bits());
        assert_ne!(x0_lo.to_bits(), x1_lo.to_bits());

        let config = parity_config(100, 100, (epoch, epoch + 1.0), (0.0, 1.0));
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_hilo_column(
            "parity-f64-x".into(),
            &f64_column(xs.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_hilo_column(
            "parity-f64-y".into(),
            &f64_column(ys.to_vec()),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: Some("f64-source".into()),
                    series_id: "f64-residual".into(),
                    x_column: "parity-f64-x".into(),
                    y_column: "parity-f64-y".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 1.0,
                        base_shape_id: crate::data_render::shape_id(&ScatterShape::CircleFilled),
                        style_map: None,
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        for (cursor, expected_index) in [([25.0, 50.0], 0), ([75.0, 50.0], 1)] {
            let picked =
                resolve_pick(&engine, &pool, query_from_config(&config, cursor, 0.0)).unwrap();
            assert_eq!(picked.point_index, expected_index);
        }
    }

    #[test]
    fn registry_transition_failure_at_each_build_position_preserves_state_and_queries() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "pick-batch-x".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "pick-batch-y".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        for series_id in ["batch-old-0", "batch-old-1", "batch-old-2"] {
            engine
                .add_series(
                    &pool,
                    scatter_descriptor(series_id, "pick-batch-x", "pick-batch-y"),
                )
                .unwrap();
        }
        let snapshot = PickEngineSnapshot::capture(&engine);
        let baseline = resolve_pick(&engine, &pool, test_query([50.0, 50.0])).unwrap();
        assert_eq!(baseline.series_id, "batch-old-2");

        for failure_at in 0..3 {
            let final_slots = (0..3)
                .map(|gpu_index| {
                    GpuPickRegistrySlot::Build(scatter_descriptor(
                        &format!("batch-next-{failure_at}-{gpu_index}"),
                        if gpu_index == failure_at {
                            "pick-batch-missing"
                        } else {
                            "pick-batch-x"
                        },
                        "pick-batch-y",
                    ))
                })
                .collect::<Vec<_>>();
            let error = match engine.prepare_registry_transition(&pool, final_slots) {
                Ok(_) => panic!("transition unexpectedly prepared with failure at {failure_at}"),
                Err(error) => error,
            };
            assert!(
                matches!(error, GpuPickError::MissingColumn(ref id) if id == "pick-batch-missing")
            );
            snapshot.assert_unchanged(&engine);
            assert_eq!(
                resolve_pick(&engine, &pool, test_query([50.0, 50.0])),
                Some(baseline.clone())
            );
        }
    }

    #[test]
    fn registry_transition_rejects_foreign_pool_before_installing_replacement() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut old_pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, value) in [
            ("pick-rf-ax", 2.0),
            ("pick-rf-ay", 5.0),
            ("pick-rf-bx", 8.0),
            ("pick-rf-by", 5.0),
        ] {
            old_pool
                .add_column(
                    id.into(),
                    &f32_column(vec![value]),
                    crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
        }
        let mut provisional_pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, value) in [
            ("pick-rf-ax", 3.0),
            ("pick-rf-ay", 5.0),
            ("pick-rf-bx", 8.0),
        ] {
            provisional_pool
                .add_column(
                    id.into(),
                    &f32_column(vec![value]),
                    crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
        }

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(
                &old_pool,
                scatter_descriptor("rebind-old-a", "pick-rf-ax", "pick-rf-ay"),
            )
            .unwrap();
        engine
            .add_series(
                &old_pool,
                scatter_descriptor("rebind-old-b", "pick-rf-bx", "pick-rf-by"),
            )
            .unwrap();
        let snapshot = PickEngineSnapshot::capture(&engine);
        let baseline = resolve_pick(&engine, &old_pool, test_query([80.0, 50.0]));

        let error = match engine.prepare_registry_transition(
            &provisional_pool,
            [GpuPickRegistrySlot::Build(scatter_descriptor(
                "rebind-next-a",
                "pick-rf-ax",
                "pick-rf-ay",
            ))],
        ) {
            Ok(_) => panic!("transition unexpectedly accepted a foreign pool"),
            Err(error) => error,
        };
        assert!(matches!(error, GpuPickError::ForeignColumnPool));
        snapshot.assert_unchanged(&engine);
        assert_eq!(
            resolve_pick(&engine, &old_pool, test_query([80.0, 50.0])),
            baseline
        );
    }

    #[test]
    fn registry_transition_replaces_affected_slots_and_rebinds_survivors() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut old_pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        old_pool
            .add_column(
                "pick-success-layout-prefix".into(),
                &f32_column(vec![0.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        for index in 0..5 {
            old_pool
                .add_column(
                    format!("pick-success-x{index}"),
                    &f32_column(vec![index as f32 + 1.0]),
                    crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
            old_pool
                .add_column(
                    format!("pick-success-y{index}"),
                    &f32_column(vec![5.0]),
                    crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
                )
                .unwrap();
        }

        let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
        for index in 0..5 {
            engine
                .add_series(
                    &old_pool,
                    scatter_descriptor(
                        &format!("success-old-{index}"),
                        &format!("pick-success-x{index}"),
                        &format!("pick-success-y{index}"),
                    ),
                )
                .unwrap();
        }
        let before_metadata = engine
            .registry
            .slots
            .iter()
            .map(PickSeriesMetadataSnapshot::capture)
            .collect::<Vec<_>>();
        let before_resources = engine
            .registry
            .slots
            .iter()
            .map(PickSeriesResourceSnapshot::capture)
            .collect::<Vec<_>>();
        let before_identities = Arc::clone(&engine.registry.identities);
        let before_reduce = engine.registry.reduce.candidates.clone();
        assert!(
            old_pool
                .remove_column("pick-success-layout-prefix")
                .unwrap()
        );
        assert!(
            old_pool
                .defragment(crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        old_pool
            .upsert_column(
                "pick-success-x0".into(),
                &f32_column(vec![1.5]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        old_pool
            .upsert_column(
                "pick-success-x2".into(),
                &f32_column(vec![3.5]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();

        let final_slots = (0usize..5)
            .map(|gpu_index| {
                if [0usize, 2, 4].contains(&gpu_index) {
                    GpuPickRegistrySlot::Build(scatter_descriptor(
                        &format!("success-next-{gpu_index}"),
                        &format!("pick-success-x{gpu_index}"),
                        &format!("pick-success-y{gpu_index}"),
                    ))
                } else {
                    let identity = &engine.registry.slots[gpu_index].identity;
                    GpuPickRegistrySlot::Reuse {
                        current_index: gpu_index,
                        source_id: identity.source_id.clone(),
                        series_id: identity.series_id.clone(),
                    }
                }
            })
            .collect::<Vec<_>>();
        engine
            .prepare_registry_transition(&old_pool, final_slots)
            .unwrap()
            .commit();

        assert_eq!(engine.registry.slots.len(), 5);
        assert!(!Arc::ptr_eq(
            &before_identities,
            &engine.registry.identities
        ));
        assert_eq!(engine.registry.reduce.candidates, before_reduce);
        assert_eq!(
            engine
                .registry
                .slots
                .iter()
                .map(|slot| slot.identity.series_id.as_str())
                .collect::<Vec<_>>(),
            [
                "success-next-0",
                "success-old-1",
                "success-next-2",
                "success-old-3",
                "success-next-4",
            ]
        );

        for index in [0usize, 2, 4] {
            assert_eq!(
                engine.registry.slots[index].gpu.pool_layout_generation,
                old_pool.layout_generation()
            );
            assert_eq!(
                engine.registry.slots[index].gpu.x_allocation_epoch,
                old_pool
                    .allocation_epoch(&format!("pick-success-x{index}"))
                    .unwrap()
            );
            if index != 4 {
                assert_ne!(
                    engine.registry.slots[index].gpu.x_allocation_epoch,
                    before_metadata[index].x_allocation_epoch
                );
            }
            assert_ne!(
                engine.registry.slots[index].gpu.gate_masks,
                before_resources[index].gate_masks
            );
            assert_ne!(
                engine.registry.slots[index].gpu.workgroup_candidates,
                before_resources[index].workgroup_candidates
            );
            assert_ne!(
                engine.registry.slots[index].gpu.query_work_bg,
                before_resources[index].query_work_bg
            );
            assert_ne!(
                engine.registry.slots[index].gpu.reduce_bg,
                before_resources[index].reduce_bg
            );
            assert_ne!(
                engine.registry.slots[index].gpu.result,
                before_resources[index].result
            );
        }
        for index in [1usize, 3] {
            let slot = &engine.registry.slots[index];
            let series = &slot.gpu;
            assert_eq!(series.gate_masks, before_resources[index].gate_masks);
            assert_eq!(
                series.workgroup_candidates,
                before_resources[index].workgroup_candidates
            );
            assert_eq!(series.query_params, before_resources[index].query_params);
            assert_ne!(series.query_data_bg, before_resources[index].query_data_bg);
            assert_eq!(series.query_work_bg, before_resources[index].query_work_bg);
            assert_eq!(series.reduce_bg, before_resources[index].reduce_bg);
            assert_eq!(series.result, before_resources[index].result);
            assert_eq!(series.pool_layout_generation, old_pool.layout_generation());
            assert_eq!(
                series.x_allocation_epoch,
                before_metadata[index].x_allocation_epoch
            );
            assert_eq!(
                series.y_allocation_epoch,
                before_metadata[index].y_allocation_epoch
            );

            let mut rebound_metadata = PickSeriesMetadataSnapshot::capture(slot);
            rebound_metadata.x_handle = before_metadata[index].x_handle;
            rebound_metadata.y_handle = before_metadata[index].y_handle;
            rebound_metadata.style_index_handle = before_metadata[index].style_index_handle;
            rebound_metadata.pool_layout_generation = before_metadata[index].pool_layout_generation;
            rebound_metadata.style_index_base = before_metadata[index].style_index_base;
            assert_eq!(rebound_metadata, before_metadata[index]);
            assert!(same_storage(
                series.x_handle,
                old_pool
                    .handle_for(&series.x_column)
                    .expect("rebound x handle")
            ));
            assert!(same_storage(
                series.y_handle,
                old_pool
                    .handle_for(&series.y_column)
                    .expect("rebound y handle")
            ));
        }

        let picked = resolve_pick(&engine, &old_pool, test_query([50.0, 50.0])).unwrap();
        assert_eq!(picked.series_id, "success-next-4");
        assert_eq!(picked.point_index, 0);
    }

    #[test]
    fn insertion_preserves_position_allows_tail_and_is_failure_atomic() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "pick-ix".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "pick-iy".into(),
            &f32_column(vec![5.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(&pool, scatter_descriptor("first", "pick-ix", "pick-iy"))
            .unwrap();
        engine
            .add_series(&pool, scatter_descriptor("last", "pick-ix", "pick-iy"))
            .unwrap();
        let inserted = engine
            .insert_series_at(1, &pool, scatter_descriptor("middle", "pick-ix", "pick-iy"))
            .unwrap();
        assert_eq!(inserted.index(), 1);

        let tied = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tied.series_id, "last");
        engine.remove_series_at(2).unwrap();
        let middle = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(middle.series_id, "middle");

        assert!(matches!(
            engine.insert_series_at(
                0,
                &pool,
                scatter_descriptor("missing", "pick-missing", "pick-iy"),
            ),
            Err(GpuPickError::MissingColumn(_))
        ));
        assert!(matches!(
            engine.insert_series_at(
                3,
                &pool,
                scatter_descriptor("invalid", "pick-ix", "pick-iy"),
            ),
            Err(GpuPickError::InvalidSeriesIndex { index: 3, len: 2 })
        ));
        let unchanged = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(unchanged.series_id, "middle");

        let appended = engine
            .insert_series_at(2, &pool, scatter_descriptor("tail", "pick-ix", "pick-iy"))
            .unwrap();
        assert_eq!(appended.index(), 2);
        let tail = pollster::block_on(
            engine
                .pick(&pool, test_query([50.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tail.series_id, "tail");
    }

    #[test]
    fn registry_mutation_preserves_order_and_clears_stale_reduction_slots() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        for (id, values) in [
            ("pick-ax", vec![2.0]),
            ("pick-ay", vec![5.0]),
            ("pick-bx", vec![8.0]),
            ("pick-by", vec![5.0]),
            ("pick-cx", vec![8.0]),
            ("pick-cy", vec![5.0]),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }

        let mut engine = GpuPickEngine::new(device, queue).unwrap();
        engine
            .add_series(&pool, scatter_descriptor("a", "pick-ax", "pick-ay"))
            .unwrap();
        engine
            .add_series(&pool, scatter_descriptor("b", "pick-bx", "pick-by"))
            .unwrap();

        let replacement = engine
            .replace_series_at(0, &pool, scatter_descriptor("c", "pick-cx", "pick-cy"))
            .unwrap();
        assert_eq!(replacement.index(), 0);
        let tied = pollster::block_on(
            engine
                .pick(&pool, test_query([80.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(tied.series_id, "b");

        engine.remove_series_at(1).unwrap();
        let no_stale = pollster::block_on(
            engine
                .pick(&pool, test_query([20.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap();
        assert!(no_stale.is_none());

        let remaining = pollster::block_on(
            engine
                .pick(&pool, test_query([80.0, 50.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(remaining.series_id, "c");
        assert_eq!(remaining.source_id.as_deref(), Some("source-c"));

        engine.clear_series();
        assert!(
            pollster::block_on(
                engine
                    .pick(&pool, test_query([80.0, 50.0]))
                    .unwrap()
                    .resolve()
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn relocation_rebind_keeps_gate_storage_and_refreshes_all_column_bases() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            64 * 1024,
        )
        .unwrap();
        pool.add_column(
            "pick-hole".into(),
            &f32_column(vec![0.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        for (id, values) in [
            ("pick-rx", vec![4.0]),
            ("pick-ry", vec![6.0]),
            ("pick-rstyle", vec![1.0]),
        ] {
            pool.add_column(
                id.into(),
                &f32_column(values),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        }
        let slots = [
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [0.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
            },
            ScatterStyleSlotGpu {
                color_premul: [0.0; 4],
                meta: [3.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
            },
        ];
        let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
        engine
            .add_series(
                &pool,
                GpuPickSeriesDescriptor {
                    source_id: None,
                    series_id: "relocated".into(),
                    x_column: "pick-rx".into(),
                    y_column: "pick-ry".into(),
                    scatter: Some(GpuPickScatter {
                        base_radius_px: 0.0,
                        base_shape_id: 5,
                        style_map: Some(GpuPickScatterStyle {
                            style_index_column: Some("pick-rstyle".into()),
                            style_slots: &slots,
                            style_overrides: &[],
                            style_meta: ScatterStyleMapMeta {
                                style_count: 2,
                                override_count: 0,
                                has_index: 1,
                                _pad: 0,
                            },
                        }),
                        size_map: None,
                    }),
                    line_width_px: None,
                    line_interpolation: crate::data_config::LineInterpolation::Linear,
                    line_style: crate::line::LineStylePreset::Solid,
                    vector: None,
                },
            )
            .unwrap();

        let before = pollster::block_on(
            engine
                .pick(&pool, test_query([40.0, 40.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(before.series_id, "relocated");
        let x_epoch = pool.allocation_epoch("pick-rx").unwrap();
        let y_epoch = pool.allocation_epoch("pick-ry").unwrap();
        let style_epoch = pool.allocation_epoch("pick-rstyle").unwrap();

        assert!(pool.remove_column("pick-hole").unwrap());
        assert!(
            pool.defragment(crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue))
                .unwrap()
        );
        assert!(matches!(
            engine.pick(&pool, test_query([40.0, 40.0])),
            Err(GpuPickError::StaleColumn { .. })
        ));
        engine.rebind_columns(&pool).unwrap();

        let after = pollster::block_on(
            engine
                .pick(&pool, test_query([40.0, 40.0]))
                .unwrap()
                .resolve(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(after.series_id, "relocated");
        assert_eq!(after.point_index, before.point_index);
        assert_eq!(pool.allocation_epoch("pick-rx"), Some(x_epoch));
        assert_eq!(pool.allocation_epoch("pick-ry"), Some(y_epoch));
        assert_eq!(pool.allocation_epoch("pick-rstyle"), Some(style_epoch));

        let gate_masks = engine.registry.slots[0].gpu.gate_masks.clone();
        let workgroup_candidates = engine.registry.slots[0].gpu.workgroup_candidates.clone();
        let layout_generation = pool.layout_generation();
        let old_style_handle = pool.handle_for("pick-rstyle").unwrap();
        assert!(pool.remove_column("pick-rstyle").unwrap());
        let new_style_handle = pool
            .add_column(
                "pick-rstyle".into(),
                &f32_column(vec![1.0]),
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(new_style_handle.offset, old_style_handle.offset);
        assert_eq!(new_style_handle.byte_size, old_style_handle.byte_size);
        assert_eq!(new_style_handle.len_values, old_style_handle.len_values);
        assert_ne!(pool.allocation_epoch("pick-rstyle"), Some(style_epoch));
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
        assert_eq!(
            engine.registry.slots[0].gpu.workgroup_candidates,
            workgroup_candidates
        );
        assert!(matches!(
            engine.pick(&pool, test_query([40.0, 40.0])),
            Err(GpuPickError::StaleColumn { column_id, .. })
                if column_id == "pick-rstyle"
        ));
    }

    #[test]
    fn gate_dispatch_covers_u32_point_space_with_two_dimensions() {
        let max_groups = 65_535;
        for n in [
            1,
            31,
            32,
            33,
            2_048,
            2_049,
            1_000_000,
            100_000_000,
            u32::MAX,
        ] {
            let (words, groups, x, y) = gate_dispatch_layout(n, max_groups).unwrap();
            assert_eq!(words, n.div_ceil(GPU_PICK_GATE_WORD_POINTS));
            assert_eq!(groups, words.div_ceil(GPU_PICK_WORKGROUP_SIZE));
            assert!(x <= max_groups);
            assert!(y <= max_groups);
            assert!(u64::from(x) * u64::from(y) >= u64::from(groups));
        }
        let (words, groups, _, _) = gate_dispatch_layout(100_000_000, max_groups).unwrap();
        assert_eq!(u64::from(words) * GATE_MASK_BYTES, 25_000_000);
        assert_eq!(u64::from(groups) * CANDIDATE_BYTES, 1_562_528);
        assert_eq!(
            GPU_PICK_DIRECT_SCAN_POINTS,
            GPU_PICK_GATE_WORD_POINTS * GPU_PICK_WORKGROUP_SIZE * 32
        );
    }

    #[test]
    fn originating_gate_word_owns_every_boundary_segment_once() {
        let n = 130u32;
        let mut starts = Vec::new();
        for first in (0..n).step_by(GPU_PICK_GATE_WORD_POINTS as usize) {
            let owned = GPU_PICK_GATE_WORD_POINTS.min(n - first);
            let segment_count = owned.min(n - 1 - first);
            starts.extend(first..first + segment_count);
        }
        assert_eq!(starts, (0..n - 1).collect::<Vec<_>>());
        assert!(starts.contains(&63));
        assert!(starts.contains(&64));
        assert!(starts.contains(&127));
    }

    #[test]
    fn split_linear_projection_preserves_independent_hi_lo_subtraction() {
        // Deliberately anti-correlated hi/lo lanes: recombining before
        // subtracting the large epoch would lose the residual.
        let values = [
            (1_700_000_000_000.0_f64 as f32, -65_535.75_f32),
            (1_700_000_000_000.0_f64 as f32, -65_535.0_f32),
            (1_700_000_131_072.0_f64 as f32, 0.125_f32),
            (1_700_000_131_072.0_f64 as f32, 0.875_f32),
        ];
        let min_hi = values.iter().map(|v| v.0).fold(f32::INFINITY, f32::min);
        let max_hi = values.iter().map(|v| v.0).fold(f32::NEG_INFINITY, f32::max);
        let min_lo = values.iter().map(|v| v.1).fold(f32::INFINITY, f32::min);
        let max_lo = values.iter().map(|v| v.1).fold(f32::NEG_INFINITY, f32::max);
        let axis_min_hi = min_hi;
        let axis_min_lo = min_lo - 1.0;
        let lower = (min_hi - axis_min_hi) + (min_lo - axis_min_lo);
        let upper = (max_hi - axis_min_hi) + (max_lo - axis_min_lo);
        for (hi, lo) in values {
            let numerator = (hi - axis_min_hi) + (lo - axis_min_lo);
            assert!(numerator >= lower && numerator <= upper);
        }
        // A negative (inverted) range reverses but does not invalidate the
        // interval after endpoint sorting.
        let inverted = [lower / -10.0, upper / -10.0];
        let inverted_lo = inverted[0].min(inverted[1]);
        let inverted_hi = inverted[0].max(inverted[1]);
        for (hi, lo) in values {
            let t = ((hi - axis_min_hi) + (lo - axis_min_lo)) / -10.0;
            assert!(t >= inverted_lo && t <= inverted_hi);
        }
    }

    #[test]
    fn style_cross_product_extent_is_conservative() {
        let scatter = GpuPickScatter {
            base_radius_px: 2.0,
            base_shape_id: 5,
            style_map: Some(GpuPickScatterStyle {
                style_index_column: None,
                style_slots: &[ScatterStyleSlotGpu {
                    color_premul: [0.0; 4],
                    meta: [10.0, 0.0, STYLE_MASK_RADIUS as f32, 0.0],
                }],
                style_overrides: &[ScatterStyleOverrideGpu {
                    point_index: 7,
                    _pad: [0; 3],
                    color_premul: [0.0; 4],
                    meta: [0.0, 2.0, STYLE_MASK_SHAPE as f32, 0.0],
                }],
                style_meta: ScatterStyleMapMeta {
                    style_count: 1,
                    override_count: 1,
                    has_index: 0,
                    _pad: 0,
                },
            }),
            size_map: None,
        };
        let extent = style_max_extent("s", &scatter).unwrap();
        assert_eq!(extent, 10.0 * MAX_SHAPE_SCALE);
    }

    #[test]
    fn shader_pipelines_compile_when_a_gpu_is_available() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let engine = GpuPickEngine::new(device.clone(), queue);
        assert!(engine.is_ok());
        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    #[test]
    fn unrelated_remove_keeps_gate_storage_while_reused_source_epoch_stales_it() {
        let Some((device, queue)) = crate::data_render::shared_device() else {
            return;
        };
        let mut pool = ColumnPool::new(
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            4 * 256,
        )
        .unwrap();
        let values = f32_column(vec![5.0]);

        let original_x = pool
            .add_column(
                "epoch-pick-x".into(),
                &values,
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        let original_x_epoch = pool.allocation_epoch("epoch-pick-x").unwrap();
        pool.add_column(
            "epoch-pick-y".into(),
            &values,
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();
        pool.add_column(
            "epoch-pick-unrelated".into(),
            &f32_column(vec![0.0]),
            crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
        )
        .unwrap();

        let mut engine = GpuPickEngine::new(Arc::clone(&device), Arc::clone(&queue)).unwrap();
        engine
            .add_series(
                &pool,
                scatter_descriptor("epoch-pick-series", "epoch-pick-x", "epoch-pick-y"),
            )
            .unwrap();
        let gate_masks = engine.registry.slots[0].gpu.gate_masks.clone();
        let workgroup_candidates = engine.registry.slots[0].gpu.workgroup_candidates.clone();
        let layout_generation = pool.layout_generation();
        let baseline = resolve_pick(&engine, &pool, test_query([50.0, 50.0]));
        assert!(baseline.is_some());

        assert!(pool.remove_column("epoch-pick-unrelated").unwrap());
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
        assert_eq!(
            engine.registry.slots[0].gpu.workgroup_candidates,
            workgroup_candidates
        );
        assert!(GpuPickEngine::validate_series_columns(&pool, &engine.registry.slots[0]).is_ok());
        assert_eq!(
            resolve_pick(&engine, &pool, test_query([50.0, 50.0])),
            baseline
        );

        assert!(pool.remove_column("epoch-pick-x").unwrap());
        let replacement_x = pool
            .add_column(
                "epoch-pick-x".into(),
                &values,
                crate::data_render::GpuAllocCtx::unbudgeted(&device, &queue),
            )
            .unwrap();
        assert_eq!(replacement_x.offset, original_x.offset);
        assert_eq!(replacement_x.len_values, original_x.len_values);
        assert_ne!(
            pool.allocation_epoch("epoch-pick-x"),
            Some(original_x_epoch)
        );
        assert_eq!(pool.layout_generation(), layout_generation);
        assert_eq!(engine.registry.slots[0].gpu.gate_masks, gate_masks);
        assert_eq!(
            engine.registry.slots[0].gpu.workgroup_candidates,
            workgroup_candidates
        );
        assert!(matches!(
            GpuPickEngine::validate_series_columns(&pool, &engine.registry.slots[0]),
            Err(GpuPickError::StaleColumn { column_id, .. })
                if column_id == "epoch-pick-x"
        ));
    }

    /// Every assembled pick module is its fragments concatenated in the
    /// declared order, each fragment verbatim, and every fragment file under
    /// `src/gpu_pick/` is in some module; the twinned blocks live in the
    /// fragments `shader_consistency` compares.
    #[test]
    fn assembled_pick_shaders_are_their_fragments_concatenated() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/gpu_pick");
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
        let mut used = std::collections::HashSet::new();
        for shader in std::iter::once(PickShader::Gate).chain(PickShader::EXACT) {
            let mut expected = String::new();
            for name in shader.fragments() {
                expected.push_str(&read(name));
                expected.push('\n');
                used.insert(*name);
            }
            assert_eq!(shader.source(), expected, "{shader:?}");
            assert_eq!(shader.fragments()[0], "core.wgsl", "{shader:?}");
            let last = *shader.fragments().last().unwrap();
            if shader == PickShader::Gate {
                assert_eq!(last, "gate_entries.wgsl");
            } else {
                assert_eq!(last, "exact_entry.wgsl", "{shader:?}");
                if let Err(error) = wgpu::naga::front::wgsl::parse_str(shader.source()) {
                    panic!("{shader:?}: {}", error.emit_to_string(shader.source()));
                }
            }
        }
        for entry in std::fs::read_dir(&dir).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            assert!(used.contains(name.as_str()), "{name} is in no pick module");
        }
    }

    /// Compile cost of the point picker per module, entry point and exact
    /// variant: the wall time of `create_shader_module` and of
    /// `create_compute_pipeline(layout: None)` per entry, three rounds, each
    /// on a fresh device. `FIGGY_PICK_TIMING_SOURCE` times one other
    /// single-module `gpu_pick.wgsl` (an older revision) instead. Run with
    /// `--ignored --nocapture`.
    #[test]
    #[ignore = "timing report"]
    fn pick_shader_compile_timings() {
        use super::exact_variant as v;
        use std::time::Instant;
        let override_source = std::env::var("FIGGY_PICK_TIMING_SOURCE")
            .ok()
            .map(|path| std::fs::read_to_string(path).expect("timing source"));
        // (module label, source, [(entry, variant label, PICK_VARIANT)]).
        type Entries = Vec<(&'static str, &'static str, Option<u32>)>;
        let mut modules: Vec<(&str, String, Entries)> = Vec::new();
        let gate_entries: Entries = vec![
            ("pick_gate_x", "-", None),
            ("pick_gate_y", "-", None),
            ("pick_reduce_candidates", "-", None),
        ];
        if let Some(source) = override_source {
            let mut entries = gate_entries;
            entries.push(("pick_exact_candidates", "default", None));
            modules.push(("single module", source, entries));
        } else {
            let exact = |variants: &[(&'static str, u32)]| -> Entries {
                variants
                    .iter()
                    .map(|(label, bits)| ("pick_exact_candidates", *label, Some(*bits)))
                    .collect()
            };
            modules.push(("gate", PickShader::Gate.source().to_string(), gate_entries));
            modules.push((
                "marker",
                PickShader::Marker.source().to_string(),
                exact(&[
                    ("precise", 0),
                    ("precise circle", 0x1000 | 5),
                    ("precise star", 0x1000 | 25),
                    ("polar", v::POLAR),
                    ("styled", v::STYLED_MARKER),
                    ("all", v::ALL),
                ]),
            ));
            modules.push((
                "marker+line",
                PickShader::MarkerLine.source().to_string(),
                exact(&[
                    ("precise", 0),
                    ("precise circle", 0x1000 | 5),
                    ("dashed", v::DASH),
                    ("styled", v::STYLED_MARKER | v::STYLED_LINE),
                    ("all (streamed)", v::ALL),
                ]),
            ));
            modules.push((
                "line",
                PickShader::Line.source().to_string(),
                exact(&[
                    ("precise", 0),
                    ("dashed", v::DASH),
                    ("styled", v::STYLED_LINE),
                    ("all", v::ALL),
                ]),
            ));
            modules.push((
                "vector",
                PickShader::Vector.source().to_string(),
                exact(&[("precise", 0)]),
            ));
        }
        for round in 0..3 {
            let (device, _queue) = crate::test_gpu::device();
            for (name, source, entries) in &modules {
                let started = Instant::now();
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(name),
                    source: wgpu::ShaderSource::Wgsl(source.clone().into()),
                });
                println!(
                    "PICK_TIMING round {round} [{name}] create_shader_module {:.1} ms",
                    started.elapsed().as_secs_f64() * 1e3
                );
                for (entry, label, variant) in entries {
                    // 0x1000 | id: the precise variant with `PICK_SHAPE = id`.
                    let constants = variant.map(|bits| {
                        if bits & 0x1000 != 0 {
                            vec![(v::CONSTANT, 0.0), ("PICK_SHAPE", f64::from(bits & 0xfff))]
                        } else {
                            vec![(v::CONSTANT, f64::from(bits))]
                        }
                    });
                    let started = Instant::now();
                    let _pipeline =
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(entry),
                            layout: None,
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: wgpu::PipelineCompilationOptions {
                                constants: constants.as_deref().unwrap_or(&[]),
                                zero_initialize_workgroup_memory: true,
                            },
                            cache: None,
                        });
                    println!(
                        "PICK_TIMING round {round} [{name}] {entry} [{label}] {:.1} ms",
                        started.elapsed().as_secs_f64() * 1e3
                    );
                }
            }
        }
    }
}
